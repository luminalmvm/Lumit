//! A member's end of a shared project that is kept online. Lumit's server
//! holds the project, sealed so that it cannot read it, and puts everyone's
//! edits in order where a host's own Lumit would.
//!
//! Every machine is a guest of that order, the host's included. It applies
//! the others' edits as they come and sends its own as they sit on the
//! document it has heard, naming the last edit it has. The server takes them
//! only if that is still the last, so exactly one machine lands each edit and
//! everyone else applies the result. When the server is lost a machine works
//! on, and merges on the way back as a guest does without its host.
//!
//! Each batch of edits says who made it and when, sealed with the edits, and
//! from those a machine keeps the list of who changed what ([`Sitting`]).
//! A batch can carry notes too ([`Note`]): words someone left, which take
//! their place in the order as edits do and are no part of the document.
//!
//! Edits are what is stored, so every machine in a project has to run the
//! Lumit that keeps it. A newer Lumit on the host's machine may move the
//! project forward to itself ([`forward`]), and everyone else follows.
//!
//! A second invite can only look ([`Member::look_invite`]). Its key is one
//! way from the secret of the link that edits, and opens everything that
//! reads the project. The server knows the two apart and takes no edit,
//! snapshot or footage from it, and the machine that holds it sends none.
//! It is off until the host turns it on ([`Member::look_body`]): until
//! then the server has nothing of it, and it opens nothing.
//!
//! Nothing here speaks to the server itself. The interface keeps a door on
//! this machine, as it does for Lumit's relay, and this dials the door.
//!
//! Threads: one reads and reconnects, each connection has a writer, and a
//! new snapshot is stored from a thread of its own.

use crate::activity::{self, Activity, By, Note, Sitting, NOTES_A_BATCH, NOTE_LENGTH};
use crate::guest::{keep, left, resolve, token, Left, LONGEST_WAIT};
use crate::host::VERSION;
use crate::invite::{locked_key, take, to_letters, Lock};
use crate::kept::{self, Finding, Kept};
use crate::local::{place, sane, sane_document, settle};
use crate::wire::{scrubbed, unscrubbed, Names, DOCUMENT_LIMIT, GREETING};
use crate::{
    Conflict, Ending, Event, Events, Invite, Person, Presence, Refusal, ShareError, Versions,
    MAX_PEOPLE, ONLINE_DOOR,
};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use lumit_core::ops::apply;
use lumit_core::store::{Moved, RemoteTag, Tap};
use lumit_core::{CompTime, Document, DocumentStore, Op};
use lumit_project::{semver_triple, SCHEMA_VERSION};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Raised whenever a frame, or what is sealed in one, changes shape.
const PROTOCOL: u32 = 1;

/// What the first line said to the door starts with.
const CALL: &str = "LUMIT-ONLINE";

// What is asked of the door, and what it answers. Anything it says that is
// not one of these or of the server's own is taken for the line having gone,
// which is also what it says when it has.
const CREATE: u8 = 0x20;
const META: u8 = 0x21;
const GET_SNAPSHOT: u8 = 0x22;
const PUT_SNAPSHOT: u8 = 0x23;
const PART: u8 = 0x24;
const END: u8 = 0x25;
const LIVE: u8 = 0x26;
const OK: u8 = 0x30;
const META_IS: u8 = 0x31;
const PART_IS: u8 = 0x32;
const END_IS: u8 = 0x33;
const AGAIN_IS: u8 = 0x34;
const LOST: u8 = 0x3e;
const FAILED: u8 = 0x3f;

// What crosses the server's own socket once a connection is live. The door
// passes these on as they are.
const APPEND: u8 = 0x01;
const PRESENCE: u8 = 0x03;
const NOTE: u8 = 0x04;
const HELLO: u8 = 0x10;
const ENTRY: u8 = 0x11;
const ACK: u8 = 0x13;
const STALE: u8 = 0x14;
const PEOPLE: u8 = 0x15;
const PRESENT: u8 = 0x16;
const NOTED: u8 = 0x17;
const CLOSED: u8 = 0x18;
const REMOVED: u8 = 0x19;
const REFUSED: u8 = 0x1a;
const MOVED: u8 = 0x1b;
const LOOKS: u8 = 0x1c;

/// What sealing puts round a piece: a nonce in front and a tag behind.
const NONCE: usize = 12;
const TAG: usize = 16;

/// The most a sealed batch comes to. The server takes no more.
const BATCH_LIMIT: usize = 64 << 20;

/// The most a batch is before it is sealed, deflated, and the most one
/// opens to. A batch that says it inflates to more is not opened.
const BATCH_PLAIN: usize = BATCH_LIMIT - NONCE - TAG;

/// The most a sealed snapshot comes to. Under the 100 MB that the platform
/// Lumit's server runs on lets one request carry, which is less than
/// 100 MiB.
const SNAPSHOT_LIMIT: usize = 95_000_000;

/// The most one frame of a snapshot carries, and what a file is sealed in
/// pieces of.
const PIECE: usize = 1 << 20;

/// The most anything else comes to: presence, a note, the list of people.
/// One message of the server's socket.
const MESSAGE_LIMIT: usize = 512 << 10;

/// How long the door has to answer, and how long a write to it may hang.
/// Longer than the door gives Lumit's server, so that what the door says of
/// a server that never answered is heard.
const ANSWER: Duration = Duration::from_secs(45);

/// How long an append too big for the socket may go unanswered before the
/// line is taken for dead. The door stores it by another way first, which is
/// as slow as the person's line. Any other append has [`ANSWER`].
// ponytail: one length whatever the size. A batch too big to cross the
// person's line in this time is sent again and again. By its size, if that
// is ever seen.
const PATIENCE: Duration = Duration::from_secs(120);

/// How often a writer looks for something to send, and how long a machine
/// rests after an append of its own went in. The rest is what lets someone
/// else's in: a machine told its append was stale sends again at once, and
/// one that never stops editing would otherwise always be first.
// ponytail: a fixed rest. A machine whose line is slower than another's by
// more than this can still be shut out for as long as the other edits
// without a break. A rest that grows with each append in a row, if seen.
const BEAT: Duration = Duration::from_millis(50);

/// How long what someone is changing is held for them after they last said
/// so, and how often this end says so again while it is changing something.
/// A machine that vanishes holds nothing for longer than the first.
const HELD: Duration = Duration::from_secs(2);
const SAID_AGAIN: Duration = Duration::from_secs(1);

/// How many frames wait for one connection's writer. Nothing the document
/// depends on is queued: an edit waits in the store, not here.
const OUTBOX: usize = 64;

/// How far past the stored snapshot the head runs before a machine stores a
/// new one: in edits, and in sealed bytes.
const COMPACT_EDITS: u64 = 2000;
const COMPACT_BYTES: u64 = 8 << 20;

/// The least a batch is cut down to for missing its place in the order:
/// what a slow line carries in under a second. No smaller, because a line
/// that is only far away gains nothing from sending less at a time.
const SMALL_BATCH: usize = 64 << 10;

/// How long a machine that has just fetched a whole snapshot goes on asking
/// to go live, with nothing more arriving, before it gives the fetch up. The
/// door goes on asking for the rest of a snapshot for twice as long.
const LINE_GONE: Duration = Duration::from_secs(60);

/// How long a machine told to wait its turn rests before it sends again.
/// Lumit's server gives whoever it told a batch was stale the next turn, and
/// tells anyone else who sends meanwhile that theirs is stale too, with the
/// head they already have.
const TURN_REST: Duration = Duration::from_millis(250);

/// How long a machine that can only look has to go without an edit being
/// taken back before the next one is said again.
const TRYING: Duration = Duration::from_secs(2);

/// How many times moving a project forward starts again because an edit
/// went in under it.
const FORWARD_TRIES: usize = 4;

/// What a sealed file starts with.
const SEALED: &[u8] = b"LUMIT-SEALED 1\n";

type Key = [u8; 32];

fn random<const N: usize>() -> Result<[u8; N], ShareError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| ShareError::NoRandomness(e.to_string()))?;
    Ok(bytes)
}

fn too_large(size: usize, limit: usize) -> ShareError {
    let (size, limit) = (size as u64, limit as u64);
    ShareError::TooLarge { size, limit }
}

/// What is said of something sealed that does not open: cut short, changed,
/// or sealed with another key.
fn spoiled() -> ShareError {
    std::io::Error::from(ErrorKind::InvalidData).into()
}

fn cipher(key: &Key) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(chacha20poly1305::Key::from_slice(key))
}

/// Seal `plain` with `key`: a random nonce, then the sealed bytes. `data` is
/// what it is and where it belongs, which is not stored with it and has to
/// be said the same to open it. That is what stops the server passing one
/// piece off as another.
fn seal(key: &Key, data: &[u8], plain: &[u8]) -> Result<Vec<u8>, ShareError> {
    let nonce: [u8; NONCE] = random()?;
    let piece = Payload {
        msg: plain,
        aad: data,
    };
    let sealed = cipher(key).encrypt(Nonce::from_slice(&nonce), piece);
    let sealed = sealed.map_err(|_| too_large(plain.len(), usize::MAX))?;
    Ok([&nonce[..], &sealed[..]].concat())
}

/// What [`seal`] sealed, or `None` for anything else.
fn open(key: &Key, data: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let (nonce, sealed) = sealed.split_at_checked(NONCE)?;
    let piece = Payload {
        msg: sealed,
        aad: data,
    };
    cipher(key).decrypt(Nonce::from_slice(nonce), piece).ok()
}

/// What a batch is sealed with besides its key: where it goes in the order.
fn batch_data(base: u64, count: u32) -> Vec<u8> {
    [&b"batch"[..], &base.to_le_bytes(), &count.to_le_bytes()].concat()
}

fn snapshot_data(seq: u64) -> Vec<u8> {
    [&b"snapshot"[..], &seq.to_le_bytes()].concat()
}

/// What an invite comes to at Lumit's server. Every step is one way, so the
/// server can work none of it back, and nor can anyone who holds less.
///
/// There are two roots. The invite's own secret, the link's, gives `read`,
/// which is the key of the invite that can only look. What finds the
/// project and reads it comes from `read`. What lets a machine store
/// anything comes from the channel key, which is that secret and the
/// password together when the host set one.
///
/// So a password guards storing and nothing else. Nothing of it is in
/// `read` or in anything worked out from `read`: the two invites held
/// together are no more than the link that edits, and give nothing to try
/// a password against. The price is that whoever holds that link can read
/// without the password, for as long as the host has the invite that can
/// only look turned on. While it is off the server keeps nothing `read`
/// opens.
struct Keys {
    /// What everything that reads the project is worked out from.
    read: Key,
    /// Names the project.
    id: String,
    /// Shown to the server on every call, which is how it knows a member
    /// and whether that member may store anything.
    access: String,
    /// What the invite that can only look shows instead. The same as
    /// `access` on a machine that holds that invite.
    look: String,
    /// Opens the key everything stored is sealed with, as the server hands
    /// it to this machine, and seals nothing else. For a machine that can
    /// edit it comes from the channel key, so it needs the password.
    wrap: Key,
    /// Seals that key again for whoever can only look. The server has this
    /// second copy only while their invite is on.
    look_wrap: Key,
    /// Seals a person's name, which is said before the stored key is known.
    name: Key,
    /// Seals the new invite for those who can edit, when the host makes
    /// one. `None` on a machine that can only look, so the server sending
    /// it the wrong one of the two hands it nothing it can open.
    told: Option<Key>,
}

/// `link` is the invite's own secret and `channel` the key its channel is
/// opened with. `look` is whether it is an invite that can only look, whose
/// one key is `read` itself.
fn keys(channel: &Key, link: &Key, look: bool) -> Keys {
    let derive = |what: &str, from: &Key| {
        let context = format!("lumit-share 2026 online {what}");
        blake3::derive_key(&context, from)
    };
    let read = if look { *channel } else { derive("look", link) };
    let looks = hex::encode(derive("look access", &read));
    let look_wrap = derive("look wrap", &read);
    Keys {
        id: hex::encode(&derive("project", &read)[..16]),
        access: match look {
            true => looks.clone(),
            false => hex::encode(derive("access", channel)),
        },
        look: looks,
        wrap: match look {
            true => look_wrap,
            false => derive("wrap", channel),
        },
        look_wrap,
        name: derive("name", &read),
        told: (!look).then(|| derive("told", channel)),
        read,
    }
}

/// What this machine tells one project kept online from another by. The
/// document's own id will not do: each copy has one of its own.
fn project_of(seal: &Key) -> Uuid {
    let name = blake3::derive_key("lumit-share 2026 online copy", seal);
    Uuid::from_slice(&name[..16]).unwrap_or_default()
}

/// Write a file by way of one beside it, so that a file at `to` is always a
/// whole one.
fn beside(
    to: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<(), ShareError>,
) -> Result<(), ShareError> {
    let name = to.file_name().unwrap_or_default().to_string_lossy();
    let making = to.with_file_name(format!("making-{name}"));
    let made = (|| {
        if let Some(folder) = to.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut file = BufWriter::new(File::create(&making)?);
        write(&mut file)?;
        file.flush()?;
        drop(file);
        Ok(fs::rename(&making, to)?)
    })();
    if made.is_err() {
        let _ = fs::remove_file(&making);
    }
    made
}

/// Read `from` to its end in pieces of `size`, handing each on with its
/// number and whether it is the last. There is always a last one, which for
/// an empty file is empty. Gives up once `stop` is set.
fn pieces(
    from: &mut impl Read,
    size: usize,
    stop: &AtomicBool,
    mut each: impl FnMut(u32, &[u8], bool) -> Result<(), ShareError>,
) -> Result<(), ShareError> {
    let mut fill = |piece: &mut Vec<u8>| {
        piece.clear();
        from.by_ref().take(size as u64).read_to_end(piece).map(drop)
    };
    let (mut piece, mut next) = (Vec::new(), Vec::new());
    fill(&mut piece)?;
    for nth in 0..=u32::MAX {
        if stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::from(ErrorKind::Interrupted).into());
        }
        // Read one ahead, which is the only way to know a piece is the last.
        fill(&mut next)?;
        each(nth, &piece, next.is_empty())?;
        if next.is_empty() {
            return Ok(());
        }
        std::mem::swap(&mut piece, &mut next);
    }
    Err(too_large(usize::MAX, size))
}

/// The nonce of piece `nth` of a sealed file: the file's own, then the
/// number. A piece moved to another place does not open there.
fn piece_nonce(file: &[u8; 8], nth: u32) -> Nonce {
    let mut nonce = [0u8; NONCE];
    nonce[..8].copy_from_slice(file);
    nonce[8..].copy_from_slice(&nth.to_le_bytes());
    nonce.into()
}

/// What seals a project's footage for Lumit's server, and opens what comes
/// back from it. Apart from the [`Member`] it was asked of, so that a long
/// file holds nothing else up.
///
/// A sealed file is a line saying what it is, a nonce of its own, and the
/// file in pieces of a megabyte, each sealed with its number and with
/// whether it is the last. So one that was cut short, or had its pieces put
/// in another order, does not open.
#[derive(Clone)]
pub struct Seal {
    key: Key,
}

impl Seal {
    /// Seal the file at `source` into a new file at `sealed`, a piece at a
    /// time. `told` hears how many bytes are done, of how many. Setting
    /// `stop` gives up, and then nothing is left at `sealed`.
    pub fn seal_file(
        &self,
        source: &Path,
        sealed: &Path,
        stop: &AtomicBool,
        told: &mut dyn FnMut(u64, u64),
    ) -> Result<(), ShareError> {
        let mut from = File::open(source)?;
        let total = from.metadata()?.len();
        let nonce: [u8; 8] = random()?;
        let cipher = cipher(&self.key);
        beside(sealed, |to| {
            to.write_all(SEALED)?;
            to.write_all(&nonce)?;
            let mut done = 0;
            pieces(&mut from, PIECE, stop, |nth, piece, last| {
                done += piece.len() as u64;
                let piece = Payload {
                    msg: piece,
                    aad: &[u8::from(last)],
                };
                let sealed = cipher.encrypt(&piece_nonce(&nonce, nth), piece);
                to.write_all(&sealed.map_err(|_| spoiled())?)?;
                told(done, total);
                Ok(())
            })
        })
    }

    /// Open the sealed file at `sealed` into a new file at `out`, a piece at
    /// a time, with `told` and `stop` as [`Self::seal_file`] has them. A file
    /// that is not whole, or not this project's, is refused, and then
    /// nothing is left at `out`.
    pub fn open_file(
        &self,
        sealed: &Path,
        out: &Path,
        stop: &AtomicBool,
        told: &mut dyn FnMut(u64, u64),
    ) -> Result<(), ShareError> {
        let from = File::open(sealed)?;
        let total = from.metadata()?.len();
        let mut from = BufReader::new(from);
        let (mut mark, mut nonce) = ([0u8; SEALED.len()], [0u8; 8]);
        from.read_exact(&mut mark)?;
        from.read_exact(&mut nonce)?;
        if mark != SEALED {
            return Err(spoiled());
        }
        let cipher = cipher(&self.key);
        beside(out, |to| {
            let mut done = (mark.len() + nonce.len()) as u64;
            pieces(&mut from, PIECE + TAG, stop, |nth, piece, last| {
                done += piece.len() as u64;
                let piece = Payload {
                    msg: piece,
                    aad: &[u8::from(last)],
                };
                let plain = cipher.decrypt(&piece_nonce(&nonce, nth), piece);
                to.write_all(&plain.map_err(|_| spoiled())?)?;
                told(done, total);
                Ok(())
            })
        })
    }
}

/// One frame: its kind, its length, and what it carries. Nothing longer than
/// a batch is ever given to this.
fn frame(kind: u8, parts: &[&[u8]]) -> Vec<u8> {
    let length: usize = parts.iter().map(|part| part.len()).sum();
    let mut bytes = Vec::with_capacity(5 + length);
    bytes.push(kind);
    bytes.extend_from_slice(&(length as u32).to_le_bytes());
    for part in parts {
        bytes.extend_from_slice(part);
    }
    bytes
}

/// Take a number off the front of `bytes`, as its bytes.
fn number<const N: usize>(bytes: &mut &[u8]) -> Option<[u8; N]> {
    take(bytes, N)?.try_into().ok()
}

/// What the door said went wrong, as the error to hand on.
fn failed(bytes: &[u8]) -> ShareError {
    #[derive(Deserialize)]
    struct Said {
        error: String,
    }
    let said = serde_json::from_slice::<Said>(bytes);
    match said
        .map_or_else(|_| String::new(), |said| said.error)
        .as_str()
    {
        // No project by that invite, or no way to the server just now.
        // Whoever is looking for one goes on looking.
        "not_found" | "offline" => ShareError::Unreachable,
        code => ShareError::Server(code.chars().take(64).collect()),
    }
}

/// A connection to the door.
struct Door {
    reader: BufReader<TcpStream>,
    socket: TcpStream,
}

impl Door {
    fn send(&mut self, kind: u8, parts: &[&[u8]]) -> Result<(), ShareError> {
        Ok(self.socket.write_all(&frame(kind, parts))?)
    }

    /// The next frame. Its length is held against what its kind may be
    /// before anything is read, and the buffer grows as the bytes arrive, so
    /// a length that lies reserves nothing.
    fn recv(&mut self) -> Result<(u8, Vec<u8>), ShareError> {
        let mut head = [0u8; 5];
        self.reader.read_exact(&mut head)?;
        let [kind, a, b, c, d] = head;
        let length = u32::from_le_bytes([a, b, c, d]) as usize;
        let limit = match kind {
            ENTRY => 16 + BATCH_LIMIT,
            PART_IS => PIECE,
            _ => MESSAGE_LIMIT,
        };
        if length > limit {
            return Err(too_large(length, limit));
        }
        let mut bytes = Vec::new();
        (&mut self.reader)
            .take(length as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() != length {
            return Err(std::io::Error::from(ErrorKind::UnexpectedEof).into());
        }
        Ok((kind, bytes))
    }

    /// Ask the door for something and wait for `answer`.
    fn ask(&mut self, kind: u8, parts: &[&[u8]], answer: u8) -> Result<Vec<u8>, ShareError> {
        self.send(kind, parts)?;
        match self.recv()? {
            (kind, bytes) if kind == answer => Ok(bytes),
            (FAILED, bytes) => Err(failed(&bytes)),
            _ => Err(ShareError::OutOfTurn),
        }
    }
}

/// What a member connects with.
struct Seat {
    /// The invite as it is handed out. Replaced when the host makes a new
    /// one, and when the person is given one for a project that has moved.
    invite: Mutex<Invite>,
    /// What the host's password came to, on the machine that set one. Its
    /// invite is not the key by itself. Anyone who joined with the password
    /// holds an invite that is.
    lock: Option<Lock>,
    name: String,
    /// What the server knows this machine's edits by from one connection to
    /// the next. Made up afresh for each store, as its edits are numbered
    /// afresh, unless edits of the last one are still in doubt.
    token: u64,
    /// The token the server last knew this copy by, for one that was closed
    /// and opened again under a new one and has not been seated since. Only
    /// what became of the copy is asked by it.
    was: Option<u64>,
    /// The folder this machine keeps the project's footage under.
    root: Option<PathBuf>,
    names: Names,
    /// The Lumit this machine runs. Always this one: it is here, and not
    /// read from the constant where it is used, so that the tests can have
    /// two of them meet.
    version: &'static str,
}

impl Seat {
    fn new(
        invite: Invite,
        lock: Option<Lock>,
        name: &str,
        token: u64,
        root: Option<PathBuf>,
    ) -> Seat {
        Seat {
            invite: Mutex::new(invite),
            lock,
            name: name.chars().take(64).collect(),
            token,
            was: None,
            root,
            names: Names::default(),
            version: VERSION,
        }
    }

    /// The key everything about the project at the server is worked out
    /// from.
    fn channel(&self) -> Key {
        let invite = self.invite.lock();
        match self.lock {
            Some(lock) if invite.locked => locked_key(&invite.link, &lock),
            _ => invite.key,
        }
    }

    /// Whether this machine holds the invite that can only look.
    fn look(&self) -> bool {
        self.invite.lock().look
    }

    /// What this seat's invite comes to at the server.
    fn keys(&self) -> Keys {
        let (link, look) = {
            let invite = self.invite.lock();
            (invite.link, invite.look)
        };
        keys(&self.channel(), &link, look)
    }
}

/// Dial the door and say which project this is about. `Unreachable` while
/// the interface keeps no door.
fn dial(seat: &Seat) -> Result<Door, ShareError> {
    dial_as(seat, seat.token)
}

/// [`dial`], as the machine the server knows by `token`.
fn dial_as(seat: &Seat, token: u64) -> Result<Door, ShareError> {
    let port = match ONLINE_DOOR.load(Ordering::Relaxed) {
        0 => return Err(ShareError::Unreachable),
        port => port,
    };
    let keys = seat.keys();
    let name = to_letters(&seal(&keys.name, b"name", seat.name.as_bytes())?);
    let at = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let socket = TcpStream::connect_timeout(&at, GREETING).map_err(|_| ShareError::Unreachable)?;
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(ANSWER))?;
    socket.set_write_timeout(Some(ANSWER))?;
    let (id, access) = (keys.id, keys.access);
    let line = format!("{CALL} {PROTOCOL} {id} {access} {token} {name}\n");
    (&socket).write_all(line.as_bytes())?;
    let reader = BufReader::new(socket.try_clone()?);
    Ok(Door { reader, socket })
}

/// What the server says of a project before anything of it is fetched.
#[derive(Deserialize)]
struct Meta {
    protocol: u32,
    version: String,
    schema: String,
    wrap: String,
    #[serde(default)]
    host: bool,
    /// Missing is yes.
    writable: Option<bool>,
    /// Whether the invite that can only look is on.
    #[serde(default)]
    looks: bool,
    /// The last sequence the server has.
    #[serde(default)]
    head: Option<u64>,
    /// The sequence its snapshot was taken at.
    #[serde(default)]
    snapshot: Option<u64>,
}

/// A door that has said the project is there.
struct Met {
    door: Door,
    seal: Key,
    host: bool,
    writable: bool,
    looks: bool,
    head: Option<u64>,
    snapshot: Option<u64>,
    /// The schema what is stored was written at.
    schema: String,
    /// How this Lumit stands to the one that keeps the project, when it is
    /// another.
    apart: Option<Versions>,
}

/// How a machine running the Lumit `mine` stands to the project `meta` is
/// of. `None` when it can join as it is. Versions are held against each
/// other by their numbers, so 0.10.0 is after 0.9.0.
///
/// A project of another protocol is refused outright, and so is one whose
/// version does not read as one: there is no saying which is the newer.
fn stands(mine: &str, meta: &Meta) -> Result<Option<Versions>, ShareError> {
    use std::cmp::Ordering::{Equal, Less};
    let version: String = meta.version.chars().take(64).collect();
    let refused = |host| ShareError::Refused(Refusal::Version { host });
    if meta.protocol != PROTOCOL {
        return Err(refused(version));
    }
    if meta.version == mine && meta.schema == SCHEMA_VERSION {
        return Ok(None);
    }
    let held = |mine, theirs| Some(semver_triple(mine)?.cmp(&semver_triple(theirs)?));
    let held = held(mine, &meta.version).zip(held(SCHEMA_VERSION, &meta.schema));
    Ok(match held {
        None => return Err(refused(version)),
        Some((Equal, Equal)) => None,
        // Behind in either, and it cannot read what the project holds.
        Some((Less, _) | (_, Less)) => Some(Versions::Update { version }),
        Some(_) if meta.host => Some(Versions::Movable { version }),
        Some(_) => Some(Versions::HostFirst { version }),
    })
}

/// Find the project a seat's invite names, whichever Lumit keeps it.
fn find(seat: &Seat) -> Result<Met, ShareError> {
    let mut door = dial(seat)?;
    let meta: Meta = serde_json::from_slice(&door.ask(META, &[], META_IS)?)?;
    let apart = stands(seat.version, &meta)?;
    let mut wrapped = [0u8; NONCE + 32 + TAG];
    hex::decode_to_slice(&meta.wrap, &mut wrapped).map_err(|_| ShareError::Unreachable)?;
    let seal = open(&seat.keys().wrap, b"wrap", &wrapped);
    let seal = seal.and_then(|seal| seal.try_into().ok());
    Ok(Met {
        door,
        seal: seal.ok_or(ShareError::Unreachable)?,
        host: meta.host,
        writable: meta.writable != Some(false),
        looks: meta.looks,
        head: meta.head,
        snapshot: meta.snapshot,
        schema: meta.schema,
        apart,
    })
}

/// Find the project a seat's invite names, to join. Not one that another
/// version of Lumit keeps: edits are what is stored, so every machine has
/// to agree on every one of them.
fn meet(seat: &Seat) -> Result<Met, ShareError> {
    let met = find(seat)?;
    match met.apart.clone() {
        Some(apart) => Err(ShareError::Versions(apart)),
        None => Ok(met),
    }
}

/// What a snapshot seals: the whole document, who changed what up to
/// there, and the notes people have left. Without the lists beside it a
/// machine that starts from a snapshot would know nothing of what was done
/// or said before it.
#[derive(Serialize)]
struct Stored<'a> {
    document: &'a Document,
    activity: &'a [Sitting],
    notes: &'a [Note],
}

/// What a batch seals: who made it, by what clock in seconds since 1970,
/// the edits, and any notes its person left. Read back loosely, as
/// [`By::of`] and [`activity::notes_of`] have it.
#[derive(Serialize)]
struct Batch<'a> {
    who: Uuid,
    name: &'a str,
    at: u64,
    ops: &'a Value,
    #[serde(skip_serializing_if = "<[Note]>::is_empty")]
    notes: &'a [Note],
}

/// How many of `ops` are edits to count. One that changes nothing is there
/// to keep a place in the order, for a note or for an edit that stays on
/// its own machine, and is nobody's work.
fn counted(ops: &[Op]) -> usize {
    let nothing = |op: &&Op| matches!(op, Op::Batch { ops } if ops.is_empty());
    ops.len() - ops.iter().filter(nothing).count()
}

/// A snapshot as it is stored: scrubbed, deflated and sealed, for its place
/// in the order.
fn sealed_snapshot(
    seal_key: &Key,
    seat: &Seat,
    document: &Document,
    (activity, notes): (&[Sitting], &[Note]),
    seq: u64,
) -> Result<Vec<u8>, ShareError> {
    let stored = Stored {
        document,
        activity,
        notes,
    };
    let json = scrubbed(&stored, &seat.names)?;
    let packed = miniz_oxide::deflate::compress_to_vec(&json, 6);
    let sealed = seal(seal_key, &snapshot_data(seq), &packed)?;
    if sealed.len() > SNAPSHOT_LIMIT {
        return Err(too_large(sealed.len(), SNAPSHOT_LIMIT));
    }
    Ok(sealed)
}

/// Store a snapshot through the door, which wants its length first. `moved`
/// is the Lumit that takes the project over with it, for the one snapshot
/// that moves a project forward, and is said after the length.
fn put_snapshot(
    door: &mut Door,
    seq: u64,
    sealed: &[u8],
    moved: Option<&str>,
    stop: &AtomicBool,
) -> Result<(), ShareError> {
    let length = sealed.len() as u64;
    let moved = moved.map(|version| {
        let schema = SCHEMA_VERSION;
        serde_json::json!({ "version": version, "schema": schema }).to_string()
    });
    let moved = moved.unwrap_or_default();
    let header = [&seq.to_le_bytes(), &length.to_le_bytes(), moved.as_bytes()];
    door.send(PUT_SNAPSHOT, &header)?;
    for part in sealed.chunks(PIECE) {
        if stop.load(Ordering::Relaxed) {
            return Err(ShareError::Unreachable);
        }
        door.send(PART, &[part])?;
    }
    door.ask(END, &[], OK).map(drop)
}

/// The server's first words on a live connection.
struct Hello {
    me: u32,
    colour: u8,
    head: u64,
    heard: u64,
    writable: bool,
}

/// Go live from `after`, the last sequence this machine has.
fn live(door: &mut Door, after: u64) -> Result<Hello, ShareError> {
    door.send(LIVE, &[&after.to_le_bytes()])?;
    let (kind, bytes) = door.recv()?;
    let mut bytes = &bytes[..];
    let hello = |bytes: &mut &[u8]| {
        Some(Hello {
            me: u32::from_le_bytes(number(bytes)?),
            colour: take(bytes, 1)?.first().copied()?,
            head: u64::from_le_bytes(number(bytes)?),
            heard: u64::from_le_bytes(number(bytes)?),
            writable: take(bytes, 1)? != [0],
        })
    };
    match kind {
        HELLO => hello(&mut bytes).ok_or(ShareError::OutOfTurn),
        REFUSED if bytes == [1] => Err(ShareError::Refused(Refusal::Full)),
        FAILED => Err(failed(bytes)),
        _ => Err(ShareError::OutOfTurn),
    }
}

/// One entry of the order, opened.
struct Entry {
    /// The sequence it ends at.
    last: u64,
    /// Whose seat it came from.
    peer: u32,
    /// The edits in it. None when the machine has had it already.
    ops: Vec<Op>,
    by: By,
    /// The notes it carries.
    notes: Vec<Note>,
}

/// Open one entry of the order, for a machine that has everything up to
/// `head`. `OutOfTurn` when it does not follow on, and `Unsafe` when it does
/// and cannot be opened or is not one this machine will hold.
fn entry(seal: &Key, seat: &Seat, mut bytes: &[u8], head: u64) -> Result<Entry, ShareError> {
    let mut header = || {
        let first = u64::from_le_bytes(number(&mut bytes)?);
        let count = u32::from_le_bytes(number(&mut bytes)?);
        let peer = u32::from_le_bytes(number(&mut bytes)?);
        let base = first.checked_sub(1)?;
        Some((base, base.checked_add(u64::from(count))?, count, peer))
    };
    let (base, last, count, peer) = header().ok_or(ShareError::OutOfTurn)?;
    if last <= head {
        let (last, ops, by) = (head, Vec::new(), By::default());
        return Ok(Entry {
            last,
            peer,
            ops,
            by,
            notes: Vec::new(),
        });
    }
    // Every machine's head is the end of a batch, so one that starts
    // anywhere else has lost its place.
    if base != head {
        return Err(ShareError::OutOfTurn);
    }
    let packed = open(seal, &batch_data(base, count), bytes).ok_or(ShareError::Unsafe)?;
    let plain = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, BATCH_PLAIN);
    let plain = plain.map_err(|_| ShareError::Unsafe)?;
    let batch = unscrubbed::<Value>(&plain, &seat.names, seat.root.as_deref());
    let mut batch = batch.map_err(|_| ShareError::Unsafe)?;
    let ops = batch.get_mut("ops").map(Value::take);
    let ops = ops.ok_or(ShareError::Unsafe)?;
    // Who and when are the sender's word for it, and only ever shown. The
    // edits are what has to be right.
    let by = By::of(&batch, &ops);
    let notes = activity::notes_of(&batch, last);
    let ops: Vec<Op> = serde_json::from_value(ops).map_err(|_| ShareError::Unsafe)?;
    if !ops.iter().all(sane) {
        return Err(ShareError::Unsafe);
    }
    Ok(Entry {
        last,
        peer,
        ops,
        by,
        notes,
    })
}

/// A connection that is live and has caught up with the server.
struct Connected {
    door: Door,
    seal: Key,
    host: bool,
    looks: bool,
    hello: Hello,
    /// The sequence of the snapshot the server has, and how many sealed
    /// bytes of entries follow it.
    snapshot: u64,
    bytes: u64,
    /// Who is here, as the server last said, if it has.
    people: Option<Vec<u8>>,
    /// Who changed what, as the snapshot and the entries after it make it,
    /// and the notes people have left.
    activity: Vec<Sitting>,
    notes: Vec<Note>,
}

/// What a machine has of the project at one place in the order: the document
/// there, and the lists as they stood. The stored snapshot is one, and so is
/// what a machine that was in the project still holds when it comes back.
struct Had {
    /// The sequence it stands at.
    head: u64,
    document: Document,
    activity: Vec<Sitting>,
    notes: Vec<Note>,
    /// The sequence of the snapshot the server has, as far as is known, and
    /// how many sealed bytes of entries have been counted since.
    snapshot: u64,
    bytes: u64,
}

/// Fetch the project from a door that has it: `had`, or the stored snapshot
/// for a machine that has nothing, and every entry after that applied in
/// order. Answers the document as the server has it now, on a connection
/// that is live from there.
fn fetch(
    met: Met,
    seat: &Seat,
    stop: &AtomicBool,
    had: Option<Had>,
) -> Result<(Connected, Document), ShareError> {
    let Met {
        mut door,
        seal,
        host,
        looks,
        schema,
        apart,
        ..
    } = met;
    let root = seat.root.as_deref();
    let fetched = had.is_none();
    let had = match had {
        Some(had) => had,
        None => fetch_snapshot(
            &mut door,
            &seal,
            seat,
            apart.is_some().then_some(&schema),
            stop,
        )?,
    };
    let Had {
        head: from,
        mut document,
        activity: mut list,
        mut notes,
        snapshot,
        bytes,
    } = had;

    let (mut head, mut bytes, mut people, mut looks) = (from, bytes, None, looks);
    // Go live from there, and take in what came after. The line can go
    // again before that is done. A machine that has just fetched the whole
    // snapshot keeps it, and asks again from where it had got to until
    // nothing more has come for [`LINE_GONE`]. One that carried on from
    // what it had has nothing to lose: whoever called asks again.
    let mut last = Instant::now();
    let hello = loop {
        let mut caught = || -> Result<Hello, ShareError> {
            let hello = live(&mut door, head)?;
            while head < hello.head {
                if stop.load(Ordering::Relaxed) {
                    return Err(ShareError::Unreachable);
                }
                match door.recv()? {
                    (ENTRY, sealed) => {
                        let entered = entry(&seal, seat, &sealed, head)?;
                        let Entry {
                            last: end, ops, by, ..
                        } = entered;
                        if counted(&ops) > 0 {
                            activity::fold(&mut list, by, counted(&ops), end);
                        }
                        for note in entered.notes {
                            activity::note(&mut notes, note);
                        }
                        for mut op in ops {
                            place(&mut op, &document, root);
                            // An edit that does not apply where its author
                            // made it is not skipped: this machine would end
                            // up on a document nobody else has.
                            apply(&mut document, &op).map_err(|_| ShareError::Unsafe)?;
                        }
                        head = end;
                        bytes += sealed.len() as u64;
                        last = Instant::now();
                    }
                    (PEOPLE, seats) => people = Some(seats),
                    (LOOKS, on) => looks = on == [1],
                    (PRESENT | NOTED, _) => {}
                    // The door, saying the server's socket has gone.
                    (LOST, _) => return Err(ShareError::Unreachable),
                    _ => return Err(ShareError::OutOfTurn),
                }
            }
            Ok(hello)
        };
        match caught() {
            Err(ShareError::Unreachable | ShareError::Io(_))
                if fetched && last.elapsed() < LINE_GONE && !stop.load(Ordering::Relaxed) =>
            {
                thread::sleep(Duration::from_secs(1));
                door = dial(seat)?;
            }
            caught => break caught?,
        }
    };
    let connected = Connected {
        door,
        seal,
        host,
        looks,
        // The head this machine reached, which is the one it goes on from.
        hello: Hello { head, ..hello },
        snapshot,
        bytes,
        people,
        activity: list,
        notes,
    };
    Ok((connected, document))
}

/// The stored snapshot, opened: what the project was at the sequence it was
/// taken at. `older` is the schema it was written at, when an older Lumit
/// than this one keeps the project.
fn fetch_snapshot(
    door: &mut Door,
    seal: &Key,
    seat: &Seat,
    older: Option<&String>,
    stop: &AtomicBool,
) -> Result<Had, ShareError> {
    let root = seat.root.as_deref();
    door.send(GET_SNAPSHOT, &[])?;
    let mut sealed = Vec::new();
    let snapshot = loop {
        match door.recv()? {
            // The door asks for the rest of a snapshot that was cut on its
            // way, so the parts of one may be a long time coming. One with
            // nothing in it is the door saying it is still asking.
            (PART_IS, part) if sealed.len() + part.len() <= SNAPSHOT_LIMIT => {
                sealed.extend_from_slice(&part);
            }
            // A newer snapshot has taken the place of the one that was
            // coming, and follows from its start.
            (AGAIN_IS, _) => sealed.clear(),
            (END_IS, seq) => break number(&mut &seq[..]).map(u64::from_le_bytes),
            (FAILED, why) => return Err(failed(&why)),
            _ => return Err(ShareError::OutOfTurn),
        }
        if stop.load(Ordering::Relaxed) {
            return Err(ShareError::Unreachable);
        }
    };
    let snapshot = snapshot.ok_or(ShareError::OutOfTurn)?;
    // Sealed with where it stands in the order, so an old one handed back
    // in a newer one's place does not open.
    let packed = open(seal, &snapshot_data(snapshot), &sealed).ok_or(ShareError::Unsafe)?;
    drop(sealed);
    let limit = DOCUMENT_LIMIT as usize;
    let json = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, limit);
    let json = json.map_err(|_| ShareError::Unsafe)?;
    let stored = unscrubbed::<Value>(&json, &seat.names, root);
    let mut stored = stored.map_err(|_| ShareError::Unsafe)?;
    drop(json);
    let document = stored.get_mut("document").map(Value::take);
    // What an older Lumit stored is walked up to this one's schema before
    // it is typed, as a file that Lumit wrote is.
    let document = document.map(|document| match older {
        Some(schema) => lumit_project::migrate(document, schema),
        None => document,
    });
    let document = document.and_then(|document| serde_json::from_value(document).ok());
    let document: Document = document.ok_or(ShareError::Unsafe)?;
    if !sane_document(&document) {
        return Err(ShareError::Unsafe);
    }
    let list = stored.get_mut("activity").map(Value::take);
    let notes = stored.get_mut("notes").map(Value::take);
    Ok(Had {
        head: snapshot,
        document,
        activity: activity::stored(list, snapshot),
        notes: activity::stored_notes(notes, snapshot),
        snapshot,
        bytes: 0,
    })
}

/// Move a project that an older Lumit keeps forward to this one: fetch what
/// is stored, build the document at the head of the order with this Lumit,
/// and store that as a new snapshot under this Lumit's version. From then on
/// the project is this version's. Everyone connected is cut off by the
/// server, and finds out how they stand when they ask for it again.
///
/// The snapshot is brought up to this Lumit's schema as a file of the older
/// Lumit's is on opening, the edits after it are applied in order, and the
/// rest of what opening a file does is done to what comes of them. If any of
/// it will not read or will not apply this stops with `Unsafe`, and nothing
/// at the server is changed: the project is still the older Lumit's, and
/// still works there. Nothing is stored that is not the whole head.
///
/// Fine to ask of a project that has been moved already. Only the host's
/// machine may: anyone else gets [`ShareError::Versions`], or `forbidden`
/// from the server.
fn carry_forward(seat: &Seat, stop: &AtomicBool) -> Result<(), ShareError> {
    // ponytail: an edit that goes in while the head is being built makes
    // the server call the snapshot stale, and it is all done again. A few
    // times and no more, so people still at work on the older Lumit can
    // hold a move off. A way to stop the server taking edits meanwhile,
    // when that happens to anyone.
    for _ in 0..FORWARD_TRIES {
        let met = find(seat)?;
        match &met.apart {
            None => return Ok(()),
            Some(Versions::Movable { .. }) => {}
            Some(apart) => return Err(ShareError::Versions(apart.clone())),
        }
        let (connected, mut document) = fetch(met, seat, stop, None)?;
        // Live only to be sent the edits. What is stored goes on a
        // connection of its own.
        let _ = connected.door.socket.shutdown(Shutdown::Both);
        lumit_project::forward_migrate(&mut document).map_err(|_| ShareError::Unsafe)?;
        let (seal, head) = (connected.seal, connected.hello.head);
        let lists = (&connected.activity[..], &connected.notes[..]);
        let sealed = sealed_snapshot(&seal, seat, &document, lists, head)?;
        let moved = Some(seat.version);
        match put_snapshot(&mut dial(seat)?, head, &sealed, moved, stop) {
            Err(ShareError::Server(code)) if code == "stale" => {}
            done => return done,
        }
    }
    Err(ShareError::OutOfTurn)
}

/// [`Member::forward`], for a project this machine has not joined:
/// [`join`] said [`Versions::Movable`] of it, and the person has agreed.
/// `name` is what the others see this person called for the moment it
/// takes. [`join`] is asked again after.
///
/// Blocks while the project is fetched and stored, so not for the UI thread.
pub fn forward(invite: Invite, name: &str) -> Result<(), ShareError> {
    if invite.locked {
        return Err(ShareError::Locked);
    }
    let seat = Seat::new(invite, None, name, token(), None);
    carry_forward(&seat, &AtomicBool::new(false))
}

/// Join the project kept online that an invite names. Answers its document
/// as it stands, with its footage pointed at this machine's copies under
/// `root` where they were found, and the connection to start editing on.
///
/// [`ShareError::Versions`] when another version of Lumit keeps it, which
/// says what would let this machine in.
///
/// Blocks while the document arrives, so not for the UI thread.
pub fn join(
    invite: Invite,
    name: &str,
    root: Option<PathBuf>,
) -> Result<(Document, Joining), ShareError> {
    if invite.locked {
        return Err(ShareError::Locked);
    }
    join_as(Seat::new(invite, None, name, token(), root))
}

/// [`join`], from the seat this machine takes.
fn join_as(seat: Seat) -> Result<(Document, Joining), ShareError> {
    let met = meet(&seat)?;
    let project = project_of(&met.seal);
    let (connected, mut document) = fetch(met, &seat, &AtomicBool::new(false), None)?;
    settle(&mut document, None, seat.root.as_deref());
    let joining = Joining {
        connected,
        seat,
        project,
    };
    Ok((document, joining))
}

/// A member that has the project's document and has not started editing it.
pub struct Joining {
    connected: Connected,
    seat: Seat,
    project: Uuid,
}

impl Joining {
    /// Start editing. `store` holds exactly the document [`join`] answered.
    pub fn start(self, store: Arc<DocumentStore>, events: Events) -> Result<Member, ShareError> {
        let inner = Inner::new(store, events, self.seat, self.project);
        // The way out before the store is shared, as a guest's is.
        let live = inner.link_up(self.connected);
        inner.store.share(true, inner.tap());
        inner.begin(live)
    }
}

/// A member's own copy, opened again.
pub struct Resuming {
    seat: Seat,
    project: Uuid,
    /// The numbers of the first edits kept, which the server may have taken.
    sent: Vec<u64>,
    /// What was kept, for a copy that closed away from the server or with
    /// edits unanswered. `None` for one that closed in step.
    left: Option<Left>,
    /// How far along the order its person had seen who changed what.
    seen: u64,
    /// Notes its person wrote that had not been sent when it closed.
    notes: Vec<Note>,
}

/// Pick up a member's copy of a project kept online where it was left.
/// `file` is the copy as its file in `folder` holds it. `None` when it is
/// not one, or the person left the project.
///
/// A copy that closed in step with the server is answered as its file has
/// it, and is replaced by whatever the server has when that is found. One
/// that closed with edits the server had not answered, or away from it,
/// comes back with those edits on it, and they are merged as a guest's are.
#[must_use]
pub fn resume(file: &mut Document, folder: &Path) -> Option<(Document, Resuming)> {
    let found = Kept::found(file.id)?;
    let online = found.invite.parse::<Invite>().ok()?.is_online();
    let in_step = found
        .online
        .as_ref()
        .is_some_and(|kept| kept.step.is_some());
    let (document, finding, left) = if online && in_step {
        (file.clone(), found, None)
    } else {
        let (document, finding, left) = left(file, folder, true)?;
        (document, finding, Some(left))
    };
    let Finding {
        project,
        invite,
        name,
        root,
        online,
        ..
    } = finding;
    let kept = online.unwrap_or_default();
    let lock = kept.lock.as_deref().and_then(Lock::from_text);
    // The token goes with the edits in doubt, which are what the server is
    // asked after by it. With none it is a new one: this store numbers its
    // edits from the start again, and by the old token the server would take
    // them for ones it has had.
    let (token, was) = match kept.sent.is_empty() {
        true => (token(), Some(kept.token)),
        false => (kept.token, None),
    };
    let seat = Seat::new(invite.parse().ok()?, lock, &name, token, root);
    let resuming = Resuming {
        seat: Seat { was, ..seat },
        project,
        sent: kept.sent,
        left,
        seen: kept.seen,
        notes: kept.notes,
    };
    Some((document, resuming))
}

impl Resuming {
    /// Carry on editing, and looking for the server. `store` holds the
    /// document [`resume`] answered.
    pub fn start(self, store: Arc<DocumentStore>, events: Events) -> Result<Member, ShareError> {
        let inner = Inner::new(store, events, self.seat, self.project);
        {
            let mut activity = inner.activity.lock();
            (activity.seen, activity.waiting) = (self.seen, self.notes);
        }
        let tap = inner.tap();
        match self.left {
            Some(left) => {
                *inner.kept.lock() = Some(left.kept);
                *inner.conflicts.lock() = left.held;
                inner.doubt.lock().clone_from(&self.sent);
                let (base, since) = (left.base, left.since);
                inner.store.share_apart_sent(tap, base, since, self.sent);
            }
            // In step when it closed, so there is nothing of its own to
            // bring: the server's document replaces it.
            None => {
                let base = inner.store.snapshot();
                inner.store.share_apart(tap, base, Vec::new());
            }
        }
        inner.begin(None)
    }
}

/// Put `store`'s project online, with this machine's person as its host.
/// Others join with [`Member::invite`]. `lock` is what [`Lock::new`] made
/// of a password everyone then has to give, or `None` for none.
///
/// The store becomes a guest of the order the server keeps, as every
/// member's is. Blocks while the document is stored, so not for the UI
/// thread. If it fails the store is left as it was.
pub fn share(
    store: Arc<DocumentStore>,
    name: &str,
    lock: Option<Lock>,
    root: Option<PathBuf>,
    events: Events,
) -> Result<Member, ShareError> {
    let invite = Invite::online(random()?, lock.as_ref());
    share_as(store, Seat::new(invite, lock, name, token(), root), events)
}

/// [`share`], from the seat this machine takes.
fn share_as(store: Arc<DocumentStore>, seat: Seat, events: Events) -> Result<Member, ShareError> {
    let seal_key: Key = random()?;
    let inner = Inner::new(store, events, seat, project_of(&seal_key));
    let document = inner.share_now().ok_or(ShareError::OutOfTurn)?;
    let put = (|| {
        let mut door = dial(&inner.seat)?;
        let keys = inner.seat.keys();
        let made = serde_json::json!({
            "protocol": PROTOCOL,
            "version": inner.seat.version,
            "schema": SCHEMA_VERSION,
            "wrap": hex::encode(seal(&keys.wrap, b"wrap", &seal_key)?),
        });
        door.ask(CREATE, &[&serde_json::to_vec(&made)?], OK)?;
        let sealed = sealed_snapshot(&seal_key, &inner.seat, &document, (&[], &[]), 0)?;
        put_snapshot(&mut door, 0, &sealed, None, &inner.stop)?;
        let hello = live(&mut door, 0)?;
        Ok::<_, ShareError>(Connected {
            door,
            seal: seal_key,
            host: true,
            // Off until the host turns it on.
            looks: false,
            hello,
            snapshot: 0,
            bytes: 0,
            people: None,
            activity: Vec::new(),
            notes: Vec::new(),
        })
    })();
    match put {
        Ok(connected) => {
            let live = inner.link_up(connected);
            inner.begin(live)
        }
        Err(e) => {
            inner.store.unshare();
            Err(e)
        }
    }
}

/// A batch on its way to the server.
struct Flight {
    /// The head it was made on.
    base: u64,
    /// The edits in it, by number, each as it was sent.
    edits: Vec<(u64, Op)>,
    /// How long it is, deflated, before sealing.
    bytes: usize,
    since: Instant,
    /// When it said it was made, and the compositions its edits name, for
    /// this machine's own list once it is answered.
    at: u64,
    comps: Vec<Uuid>,
    /// How many places in the order it takes: one for each edit, and one
    /// for a batch that is notes and no edit.
    count: u32,
    /// How many of the notes waiting here are in it, from the first.
    notes: usize,
}

impl Flight {
    /// Too big for one message of the server's socket, so the door stores
    /// it first and reads nothing else until it has.
    fn slow(&self) -> bool {
        self.bytes + NONCE + TAG + 21 > MESSAGE_LIMIT
    }

    /// Unanswered for longer than the server takes to answer.
    fn lost(&self) -> bool {
        let patience = if self.slow() { PATIENCE } else { ANSWER };
        self.since.elapsed() > patience
    }
}

/// Where this machine and the server stand with each other. One lock for
/// all of it, taken for a moment at a time, so the reader and the writer
/// each see the other's last step whole.
struct Line {
    /// Which connection this is about, to tell its writer from the last
    /// one's.
    link: u64,
    /// The last sequence applied here.
    head: u64,
    /// The last sequence the server is known to have. Nothing is sent until
    /// `head` is there: an append made on less would be stale.
    behind: u64,
    /// The number of the last edit the server has had from this machine.
    heard: u64,
    /// The batch waiting on its answer. One at a time.
    flight: Option<Flight>,
    /// Nothing is sent before this.
    rest: Instant,
    /// Whether the server is taking edits, as far as this machine knows.
    writable: bool,
    /// The most a batch may come to, halved when the server found one too
    /// large.
    limit: usize,
    /// The most the next batch is made of. Halved when one missed its place
    /// in the order, or was too big for the socket and was lost with the
    /// line, down to [`SMALL_BATCH`], and doubled when one went in. Kept
    /// from one connection to the next.
    piece: usize,
    /// The sequence of the snapshot the server has, as far as this machine
    /// knows, and how many sealed bytes of entries have gone in since.
    snapshot: u64,
    bytes: u64,
    /// The server takes nothing more until a newer snapshot is stored, and
    /// this machine has gone to store one. Nothing is sent meanwhile.
    full: bool,
}

/// What waits for a connection's writer.
enum Out {
    Frame(Vec<u8>),
    /// There may be edits to send.
    Wake,
}

struct Inner {
    store: Arc<DocumentStore>,
    events: Events,
    seat: Seat,
    /// Which project this is, worked out from the key it is sealed with.
    /// One found again has to be the same, or there is nothing to merge
    /// with.
    project: Uuid,
    /// The key everything stored is sealed with, once the server has given
    /// it or this machine has made it.
    seal: OnceLock<Key>,
    stop: AtomicBool,
    /// A new invite has come, so the wait before the next try is cut short.
    hurry: AtomicBool,
    me: AtomicU32,
    host: AtomicBool,
    /// The way to the server while there is one.
    link: Mutex<Option<(SyncSender<Out>, TcpStream)>>,
    line: Mutex<Line>,
    /// The numbers of the edits that were on their way when the server was
    /// last lost. It may have taken them all the same, and a copy that
    /// closes before finding out has to be able to ask.
    doubt: Mutex<Vec<u64>>,
    /// What this end is looking at, if the others have not been told yet.
    presence: Mutex<Option<Presence>>,
    /// What this end last said, while it is changing something, and when.
    /// It is said again every [`SAID_AGAIN`], or the others let go.
    said: Mutex<Option<(Presence, Instant)>>,
    /// When each of the others last said they are changing something.
    changing: Mutex<Vec<(u32, Instant)>>,
    people: Mutex<Vec<Person>>,
    /// Edits held back by a merge, until the person chooses.
    conflicts: Mutex<Vec<Conflict>>,
    /// The edits made since the server was lost, on disk. Taken before the
    /// store's lock, never under it.
    kept: Mutex<Option<Kept>>,
    /// The person has left, or it is over, so nothing more is kept.
    left: AtomicBool,
    /// The person is closing the copy without saving it.
    discard: AtomicBool,
    /// A new snapshot is being stored.
    packing: AtomicBool,
    /// The server turned an append away, and has not gone. There is no hurry
    /// back to it.
    barred: AtomicBool,
    /// Who changed what. Nothing else is taken while it is held.
    activity: Mutex<Activity>,
    /// How this Lumit stands to the one that keeps the project, while it is
    /// another and this machine is kept out by it.
    apart: Mutex<Option<Versions>>,
    /// When an edit made on a machine that can only look was last taken
    /// back.
    tried: Mutex<Option<Instant>>,
    /// Whether the invite that can only look is on, as the server last
    /// said.
    looks: AtomicBool,
    /// A snapshot of the project was too large for this machine to store,
    /// and none has fitted since.
    too_large: AtomicBool,
    /// The head at which the server turned this machine away for room while
    /// that was so. Nothing goes in there until someone stores a snapshot.
    walled: Mutex<Option<u64>>,
    /// The next return fetches everything: carrying on from what this
    /// machine had did not work.
    whole: AtomicBool,
}

/// A project kept online, as one of the machines in it. Leaving is
/// [`Self::leave`]. A drop is the project closing, which keeps the copy a
/// member for when it is opened again.
pub struct Member {
    inner: Arc<Inner>,
}

impl Member {
    /// Everyone here just now, this person included.
    #[must_use]
    pub fn people(&self) -> Vec<Person> {
        self.inner.people.lock().clone()
    }

    /// Which of [`Self::people`] is this machine. The server gives a new
    /// number each time it is found again.
    #[must_use]
    pub fn me(&self) -> u32 {
        self.inner.me.load(Ordering::Relaxed)
    }

    /// Tell the others what this end is looking at. Latest wins.
    pub fn set_presence(&self, presence: Presence) {
        *self.inner.presence.lock() = Some(presence.tidied());
    }

    /// Say `body` to the person numbered `to`. Let go of when there is no
    /// way to the server, or it is more than one message holds.
    pub fn note(&self, to: u32, body: serde_json::Value) {
        let sealed = self.inner.seal.get().zip(serde_json::to_vec(&body).ok());
        let sealed = sealed.and_then(|(key, body)| seal(key, b"note", &body).ok());
        if let Some(sealed) = sealed.filter(|sealed| sealed.len() + 5 < MESSAGE_LIMIT) {
            self.inner.say(frame(NOTE, &[&to.to_le_bytes(), &sealed]));
        }
    }

    /// The edits a merge held back, each waiting on [`Self::resolve`].
    #[must_use]
    pub fn conflicts(&self) -> Vec<Conflict> {
        self.inner.conflicts.lock().clone()
    }

    /// Settle the conflict at `index`, as [`crate::Guest::resolve`] does.
    pub fn resolve(&self, index: usize, mine: bool) -> usize {
        let inner = &self.inner;
        resolve(&inner.store, &inner.conflicts, &inner.kept, index, mine)
    }

    /// Find the project by a new invite from now on, for one the host made
    /// while this machine was away. Anything but an invite to a project
    /// kept online, with its password given, is passed over.
    pub fn reinvite(&self, invite: Invite) {
        if invite.is_online() && !invite.locked {
            self.inner.adopt(invite);
        }
    }

    /// Whether this machine is without the server just now.
    #[must_use]
    pub fn away(&self) -> bool {
        self.inner.link.lock().is_none()
    }

    /// Whether this is the machine of the person whose account keeps the
    /// project online, by the server's account when it was last found.
    #[must_use]
    pub fn host(&self) -> bool {
        self.inner.host.load(Ordering::Relaxed)
    }

    /// Whether the server is taking edits. See [`Event::Writable`].
    #[must_use]
    pub fn writable(&self) -> bool {
        self.inner.line.lock().writable
    }

    /// Whether this machine can only look: it joined by the invite that
    /// changes nothing. What is made here is then taken back at once, so
    /// what it shows is always the project as everyone has it.
    #[must_use]
    pub fn look(&self) -> bool {
        self.inner.seat.look()
    }

    /// The invite that can only look, to hand out beside [`Self::invite`].
    /// It opens by itself, whatever password that one needs: the password
    /// guards editing, and nobody who only looks is ever given it. On a
    /// machine that can only look it is the invite it holds. It opens
    /// nothing while [`Self::looks`] is false.
    #[must_use]
    pub fn look_invite(&self) -> Invite {
        Invite::looking(self.inner.seat.keys().read)
    }

    /// Whether the invite that can only look is on, as the server last
    /// said. A project starts with it off. See [`Event::LookInvite`].
    #[must_use]
    pub fn looks(&self) -> bool {
        self.inner.looks.load(Ordering::Relaxed)
    }

    /// Whether the project has grown too large for this machine to store a
    /// snapshot of. See [`Event::TooLarge`].
    #[must_use]
    pub fn too_large(&self) -> bool {
        self.inner.too_large.load(Ordering::Relaxed)
    }

    /// For the host: what to say to Lumit's server to turn the invite that
    /// can only look on or off. That is the body of the call that does it,
    /// which the interface makes, as it makes the one [`Self::move_to`] is
    /// for. Nothing changes here until the server says it has: every
    /// machine in the project is told, this one too.
    ///
    /// On gives the server what that invite shows and the key everything is
    /// sealed with, wrapped for it. Off has the server forget both and put
    /// out everyone who is here by it. The invite that edits is the same
    /// either way, and so is the link of the one that looks: turned on
    /// again, it opens the project again.
    ///
    /// `Unreachable` for a copy opened again that has not found the server
    /// yet, and on a machine that can only look.
    pub fn look_body(&self, on: bool) -> Result<String, ShareError> {
        let keys = self.inner.seat.keys();
        keys.told.ok_or(ShareError::Unreachable)?;
        let seal_key = self.inner.seal.get().ok_or(ShareError::Unreachable)?;
        let body = match on {
            true => serde_json::json!({
                "look": keys.look,
                "wrap": hex::encode(seal(&keys.look_wrap, b"wrap", seal_key)?),
            }),
            false => serde_json::json!({ "look": null }),
        };
        Ok(body.to_string())
    }

    /// The invite to hand out.
    #[must_use]
    pub fn invite(&self) -> Invite {
        self.inner.seat.invite.lock().clone()
    }

    /// What the interface needs to move this project's footage to and from
    /// Lumit's server by itself: the project's name there, and what it
    /// shows the server on every call.
    #[must_use]
    pub fn access(&self) -> (String, String) {
        let keys = self.inner.seat.keys();
        (keys.id, keys.access)
    }

    /// What seals this project's footage. `None` for a copy opened again
    /// that has not found the server yet.
    #[must_use]
    pub fn seal(&self) -> Option<Seal> {
        self.inner.seal.get().map(|key| Seal { key: *key })
    }

    /// What a save writes while the server is away, as
    /// [`crate::Guest::saving`] answers it.
    #[must_use]
    pub fn saving(&self) -> Option<(Arc<Document>, u64, usize)> {
        self.inner.store.saving_apart()
    }

    /// The copy was saved with the first `mark` edits made while away in it.
    pub fn saved(&self, mark: usize) {
        self.inner.keep(false);
        if let Some(kept) = self.inner.kept.lock().as_mut() {
            kept.saved = mark;
        }
    }

    /// The person chose not to save the copy they are closing. See
    /// [`crate::Guest::discard`].
    pub fn discard(&self) {
        self.inner.discard.store(true, Ordering::Relaxed);
    }

    /// For the host: a new invite to replace the one handed out before, and
    /// what to say to Lumit's server to move the project to it, taking the
    /// person numbered `person` out first if there is one. That is the body
    /// of the call that moves it, which the interface makes: it has the
    /// account the server knows the host by. Nothing changes here until
    /// [`Self::moved`] says the server took it.
    ///
    /// Everyone else here is then sent the new invite by the server. Someone
    /// away just then is not, and needs it given. What is stored is not
    /// sealed again: the removed person keeps what they had fetched and can
    /// fetch no more. `Unreachable` for a copy opened again that has not
    /// found the server yet, and on a machine that can only look.
    ///
    /// The invite that can only look is replaced with it. Whoever is here
    /// by that one is sent the new one of its kind, and whoever can edit
    /// the new one that edits, sealed with what only they hold.
    pub fn move_to(&self, person: Option<u32>) -> Result<(Invite, String), ShareError> {
        let inner = &self.inner;
        let seal_key = inner.seal.get().ok_or(ShareError::Unreachable)?;
        let editors = inner.seat.keys().told.ok_or(ShareError::Unreachable)?;
        let invite = Invite::online(random()?, inner.seat.lock.as_ref());
        let channel = match inner.seat.lock {
            Some(lock) => locked_key(&invite.link, &lock),
            None => invite.key,
        };
        let new = keys(&channel, &invite.link, false);
        // What the others are sent opens the project by itself. They have
        // given the password already.
        let told = Invite {
            key: channel,
            ..Invite::online(invite.link, None)
        }
        .to_string();
        let mut body = serde_json::json!({
            "seat": person,
            "id": new.id,
            "access": new.access,
            "wrap": hex::encode(seal(&new.wrap, b"wrap", seal_key)?),
            "moved": hex::encode(seal(&editors, b"invite", told.as_bytes())?),
        });
        // The invite that can only look stays as it was: on, with its new
        // link, or off, with nothing of it given to the server.
        if self.looks() {
            let lookers = Invite::looking(new.read).to_string();
            body["look"] = new.look.into();
            body["look_wrap"] = hex::encode(seal(&new.look_wrap, b"wrap", seal_key)?).into();
            body["moved_look"] = hex::encode(seal(seal_key, b"invite", lookers.as_bytes())?).into();
        }
        Ok((invite, body.to_string()))
    }

    /// For the host: the server moved the project to the invite
    /// [`Self::move_to`] made. It is the one handed out from now on, and the
    /// project is found by it.
    pub fn moved(&self, invite: Invite) {
        if invite.is_online() {
            self.inner.adopt(invite);
        }
    }

    /// How this machine stands to the project while another version of
    /// Lumit keeps it, as [`Event::Versions`] last said. `None` once it is
    /// back, and for a copy that has not found the server yet.
    #[must_use]
    pub fn versions(&self) -> Option<Versions> {
        self.inner.apart.lock().clone()
    }

    /// For the host's machine, when [`Self::versions`] is
    /// [`Versions::Movable`] and the person has agreed: move the project
    /// forward to this Lumit. This machine then joins by itself, and so
    /// does every other that runs this Lumit. One on the older Lumit is cut
    /// off and told to update, with the edits it had not had answered kept
    /// for when it has.
    ///
    /// `Unsafe` when this Lumit could not read what the older one stored,
    /// which leaves the project as it was. [`ShareError::Versions`] or
    /// `Server("forbidden")` when it is not this machine's to move.
    ///
    /// Answers the work, for the caller to do: it blocks while the project
    /// is fetched and stored, so it is not for the UI thread, nor for
    /// under whatever lock the member is kept behind.
    pub fn forward(&self) -> impl FnOnce() -> Result<(), ShareError> + Send + 'static {
        let inner = self.inner.clone();
        move || {
            carry_forward(&inner.seat, &inner.stop)?;
            // No waiting out the time to the next look.
            inner.hurry.store(true, Ordering::Relaxed);
            Ok(())
        }
    }

    /// Who changed what, oldest sitting first: everyone's edits as the
    /// server's order has them, and this machine's own as they were
    /// answered. Empty for a copy opened again that has not found the
    /// server yet.
    #[must_use]
    pub fn activity(&self) -> Vec<Sitting> {
        self.inner.activity.lock().list.clone()
    }

    /// The notes people have left in the project, oldest first, as the
    /// order has them, and after them the ones written here that it has
    /// not got yet, which stand at 0. Two hundred are kept.
    #[must_use]
    pub fn notes(&self) -> Vec<Note> {
        let activity = self.inner.activity.lock();
        let all = || activity.notes.iter().chain(&activity.waiting);
        // What takes a note down is not one to read, and one being taken
        // down here is gone from this list before the order has heard.
        let down: Vec<Uuid> = all().filter_map(|note| note.takes_down).collect();
        let read = |note: &&Note| note.takes_down.is_none() && !down.contains(&note.id);
        all().filter(read).cloned().collect()
    }

    /// Take a note down. Its writer may, and the host may take down
    /// anyone's. It goes into the order as the note did, so everyone loses
    /// it together, whoever is away loses it on the way back, and no
    /// snapshot brings it back. Asked while this machine is away it waits
    /// as a note does, and the note is gone from the list here at once.
    ///
    /// Who may is decided here, on the machine that asks, as who may edit
    /// is. The others take the order's word for it. False when there is no
    /// such note to read, it is someone else's and this is not the host's
    /// machine, two hundred things already wait to be sent, or this machine
    /// can only look.
    pub fn take_note_down(&self, id: Uuid) -> bool {
        let inner = &self.inner;
        if inner.seat.look() {
            return false;
        }
        let me = inner.store.snapshot().id;
        let host = inner.host.load(Ordering::Relaxed);
        let mine = self.notes().into_iter().find(|note| note.id == id);
        if !mine.is_some_and(|note| host || note.who == me) {
            return false;
        }
        // One still waiting to be sent is taken down the same way, after
        // it: it may be on its way already.
        let down = Note {
            id: Uuid::now_v7(),
            who: me,
            name: inner.seat.name.clone(),
            at: activity::now(),
            text: String::new(),
            comp: None,
            time: None,
            layer: None,
            last: 0,
            new: false,
            takes_down: Some(id),
        };
        if !inner.activity.lock().write(down) {
            return false;
        }
        if let Some(kept) = inner.kept.lock().as_mut() {
            kept.stale = true;
        }
        inner.wake();
        (inner.events)(Event::Activity);
        true
    }

    /// Leave a note in the project: some words, with the composition this
    /// person has open, where their playhead is in it, and the layer they
    /// have chosen, for each that there is. It takes its place in the order
    /// as an edit does, so everyone has it, whoever was away gets it when
    /// they are back, and it is kept through every new snapshot. Written
    /// while this machine is away, it waits and is sent on the way back,
    /// and is kept with the copy if that closes first.
    ///
    /// It is no part of the document. The words are cut to
    /// [`NOTE_LENGTH`] characters. False for no words, when two hundred
    /// already wait to be sent, and on a machine that can only look, from
    /// which the server would not take it.
    pub fn write_note(
        &self,
        text: &str,
        comp: Option<Uuid>,
        time: Option<CompTime>,
        layer: Option<Uuid>,
    ) -> bool {
        let inner = &self.inner;
        let text: String = text.trim().chars().take(NOTE_LENGTH).collect();
        if text.is_empty() || inner.seat.look() {
            return false;
        }
        let note = Note {
            id: Uuid::now_v7(),
            who: inner.store.snapshot().id,
            name: inner.seat.name.clone(),
            at: activity::now(),
            text,
            comp,
            time,
            layer,
            last: 0,
            new: false,
            takes_down: None,
        };
        if !inner.activity.lock().write(note) {
            return false;
        }
        // Written down with what else is kept, for a copy that is away.
        if let Some(kept) = inner.kept.lock().as_mut() {
            kept.stale = true;
        }
        inner.wake();
        (inner.events)(Event::Activity);
        true
    }

    /// What this machine's own sittings go by in [`Self::activity`].
    #[must_use]
    pub fn who(&self) -> Uuid {
        self.inner.store.snapshot().id
    }

    /// The person has been shown who changed what, so none of it is new to
    /// them any more. The copy remembers how far that was.
    pub fn seen(&self) {
        let head = self.inner.line.lock().head;
        self.inner.activity.lock().see(head);
        // Written again with what else is kept, or by itself for a copy
        // that is in step.
        if let Some(kept) = self.inner.kept.lock().as_mut() {
            kept.stale = true;
        }
        if !self.away() {
            self.inner.in_step();
        }
    }

    /// Leave. The document stays as it is, the copy is a member no longer,
    /// and the project stays online for everyone else.
    pub fn leave(&self) {
        self.inner.forget();
        self.inner.end();
    }
}

/// A drop is the project closing, not the person leaving. A copy in step
/// with the server notes only that it is a member. One that is away, or has
/// edits the server had not answered or a conflict the person had not, keeps
/// them as a guest's copy does.
impl Drop for Member {
    fn drop(&mut self) {
        let inner = &self.inner;
        // Said first, so a merge finishing this moment leaves what is kept.
        inner.stop.store(true, Ordering::Relaxed);
        let away = self.away();
        inner.adrift();
        let held = !inner.conflicts.lock().is_empty();
        if away || held || inner.store.unanswered() > 0 {
            inner.keep(away && inner.discard.load(Ordering::Relaxed));
        } else {
            inner.in_step();
        }
        // The last of it. The member's thread can be some seconds noticing,
        // and what it kept after this would land on the copy opened again.
        {
            let _kept = inner.kept.lock();
            inner.left.store(true, Ordering::Relaxed);
        }
        inner.end();
    }
}

impl Inner {
    fn new(store: Arc<DocumentStore>, events: Events, seat: Seat, project: Uuid) -> Arc<Self> {
        let line = Line {
            link: 0,
            head: 0,
            behind: 0,
            heard: 0,
            flight: None,
            rest: Instant::now(),
            writable: true,
            limit: BATCH_PLAIN,
            piece: BATCH_PLAIN,
            snapshot: 0,
            bytes: 0,
            full: false,
        };
        Arc::new(Inner {
            store,
            events,
            seat,
            project,
            seal: OnceLock::new(),
            stop: AtomicBool::new(false),
            hurry: AtomicBool::new(false),
            me: AtomicU32::new(0),
            host: AtomicBool::new(false),
            link: Mutex::new(None),
            line: Mutex::new(line),
            doubt: Mutex::new(Vec::new()),
            presence: Mutex::new(None),
            said: Mutex::new(None),
            changing: Mutex::new(Vec::new()),
            people: Mutex::new(Vec::new()),
            conflicts: Mutex::new(Vec::new()),
            kept: Mutex::new(None),
            left: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            packing: AtomicBool::new(false),
            barred: AtomicBool::new(false),
            activity: Mutex::new(Activity::default()),
            apart: Mutex::new(None),
            tried: Mutex::new(None),
            looks: AtomicBool::new(false),
            too_large: AtomicBool::new(false),
            walled: Mutex::new(None),
            whole: AtomicBool::new(false),
        })
    }

    /// The store's tap. Weak, or the store would hold the member that holds
    /// the store. An edit made here is not sent from the tap, which runs
    /// under the store's lock: the writer is woken and reads it from there.
    fn tap(self: &Arc<Self>) -> Tap {
        let tapped = Arc::downgrade(self);
        Arc::new(move |moved| {
            if let (Moved::Local { .. }, Some(inner)) = (moved, tapped.upgrade()) {
                inner.wake();
            }
        })
    }

    /// Start the member's thread, with a connection or looking for one. It
    /// is not waited for when the member goes: it ends by itself once it
    /// sees `stop`, which the longest it can take to see is one answer from
    /// the door.
    fn begin(self: &Arc<Self>, live: Option<Door>) -> Result<Member, ShareError> {
        let running = self.clone();
        let spawned = thread::Builder::new()
            .name("lumit-online".into())
            .spawn(move || running.run(live));
        if let Err(e) = spawned {
            self.end();
            return Err(e.into());
        }
        Ok(Member {
            inner: self.clone(),
        })
    }

    /// Make the store a guest of the order the server keeps, and answer the
    /// document it held at that moment, which is where everyone starts from.
    /// An edit made in the same moment would be in neither that document nor
    /// the edits waiting to be sent, so it is tried again round it.
    fn share_now(self: &Arc<Self>) -> Option<Arc<Document>> {
        for _ in 0..64 {
            self.store.share(true, self.tap());
            // Read before the edits are asked after, so one made in between
            // shows there.
            let document = self.store.snapshot();
            if self.store.pending().is_empty() {
                return Some(document);
            }
            self.store.unshare();
        }
        None
    }

    fn wake(&self) {
        if let Some((out, _)) = self.link.lock().as_ref() {
            let _ = out.try_send(Out::Wake);
        }
    }

    /// Say something to the server that is not an edit, if it is there.
    /// What cannot be queued is let go.
    fn say(&self, frame: Vec<u8>) {
        if let Some((out, _)) = self.link.lock().as_ref() {
            let _ = out.try_send(Out::Frame(frame));
        }
    }

    fn hang_up(&self) {
        if let Some((_, socket)) = self.link.lock().take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    /// Stop for good. The store is let go of here and not on the member's
    /// thread, which can be some seconds noticing.
    fn end(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.hang_up();
        self.store.unshare();
    }

    /// The person has left, or it is over for them: the copy is a member no
    /// longer, and nothing is kept for it.
    fn forget(&self) {
        self.left.store(true, Ordering::Relaxed);
        let mut kept = self.kept.lock();
        if let Some(kept) = kept.take() {
            kept.remove();
        }
        kept::unkeep(self.store.snapshot().id);
    }

    /// How this copy finds its project again, to keep.
    fn finding(&self) -> Finding {
        let invite = self.seat.invite.lock().to_string();
        let (name, root) = (self.seat.name.clone(), self.seat.root.clone());
        let mut finding = Finding::new(self.project, invite, name, root);
        self.seat.version.clone_into(&mut finding.version);
        let (seen, notes) = {
            let activity = self.activity.lock();
            (activity.seen, activity.waiting.clone())
        };
        // The token the server knows this copy by: this run's once it has
        // been seated, and until then the one it went by before it was
        // opened again. A copy closed before it found the server still has
        // that one to ask by the next time.
        let seated = self.me.load(Ordering::Relaxed) != 0;
        let known = self.seat.was.filter(|_| !seated);
        finding.online = Some(kept::Online {
            token: known.unwrap_or(self.seat.token),
            sent: self.doubt.lock().clone(),
            step: None,
            lock: self.seat.lock.map(|lock| lock.text()),
            seen,
            schema: SCHEMA_VERSION.to_owned(),
            notes,
        });
        finding
    }

    /// [`keep`], for this member.
    fn keep(&self, discard: bool) {
        let (store, conflicts) = (&self.store, &self.conflicts);
        keep(store, &self.kept, &self.left, conflicts, discard, || {
            self.finding()
        });
    }

    /// Note on disk that this copy is a member in step with the server, so
    /// that it is one when it is opened again, whether Lumit closes or goes
    /// down. Nothing else needs keeping: the server has every edit.
    fn in_step(&self) {
        let head = self.line.lock().head;
        let kept = self.kept.lock();
        if kept.is_some() || self.left.load(Ordering::Relaxed) {
            return;
        }
        let mut finding = self.finding();
        if let Some(online) = finding.online.as_mut() {
            online.step = Some(head);
        }
        let _ = kept::note(self.store.snapshot().id, &finding);
    }

    /// The server is lost, or the copy is closing. A batch on its way is in
    /// doubt from here, and the store keeps the edits made from now on.
    /// Under the line's lock, so that an answer being taken in is either
    /// all in or not begun.
    fn adrift(&self) {
        let mut line = self.line.lock();
        if let Some(flight) = line.flight.take() {
            // One too big for the socket is stored whole by a call of its
            // own, and has all of it to send again when the line goes under
            // it. So the next is half as much, as after one that missed its
            // place. Once it fits the socket it is not stored that way.
            if flight.slow() {
                line.piece = (flight.bytes / 2).max(SMALL_BATCH);
            }
            *self.doubt.lock() = flight.edits.iter().map(|(id, _)| *id).collect();
        }
        self.store.cast_adrift();
    }

    fn tell_people(&self, people: Vec<Person>) {
        let me = self.me.load(Ordering::Relaxed);
        (self.events)(Event::People { me, people });
    }

    /// Without the server nobody else can be seen, so the list is this
    /// person alone until it is back.
    fn alone(&self) {
        let me = self.me.load(Ordering::Relaxed);
        let people = {
            let mut people = self.people.lock();
            people.retain(|p| p.id == me);
            if people.is_empty() {
                people.push(Person {
                    id: me,
                    name: self.seat.name.clone(),
                    colour: 1,
                    presence: Presence::default(),
                    host: self.host.load(Ordering::Relaxed),
                    look: self.seat.look(),
                });
            }
            people.clone()
        };
        self.tell_people(people);
    }

    /// Who is here, from the server's list of them. Each keeps what they
    /// were last seen looking at. A name that does not open is left empty
    /// and its person kept. The server marks each: 1 for the host, 2 for
    /// someone who can only look, and 3 for both.
    fn seated(&self, mut bytes: &[u8]) -> Option<Vec<Person>> {
        let name = self.seat.keys().name;
        let count = u32::from_le_bytes(number(&mut bytes)?);
        let mut people = self.people.lock();
        let mut now = Vec::new();
        for _ in 0..count.min(MAX_PEOPLE as u32) {
            let id = u32::from_le_bytes(number(&mut bytes)?);
            let colour = take(&mut bytes, 1)?.first().copied()?;
            let mark = take(&mut bytes, 1)?.first().copied()?;
            let length = u16::from_le_bytes(number(&mut bytes)?);
            let sealed = take(&mut bytes, usize::from(length))?;
            let named = open(&name, b"name", sealed).and_then(|name| String::from_utf8(name).ok());
            let was = people.iter().find(|p| p.id == id);
            now.push(Person {
                id,
                name: named.unwrap_or_default().chars().take(64).collect(),
                colour,
                presence: was.map(|p| p.presence.clone()).unwrap_or_default(),
                host: mark & 1 != 0,
                look: mark & 2 != 0,
            });
        }
        people.clone_from(&now);
        Some(now)
    }

    /// Make a fresh connection the way out, and start its writer. Answers
    /// the connection to read from, or `None` when the writer would not
    /// start.
    fn link_up(self: &Arc<Self>, connected: Connected) -> Option<Door> {
        let Connected {
            door,
            seal,
            host,
            looks,
            hello,
            snapshot,
            bytes,
            people,
            activity,
            notes,
        } = connected;
        let _ = self.seal.set(seal);
        let me = self.store.snapshot().id;
        self.activity
            .lock()
            .replace(activity, notes, me, hello.head);
        self.me.store(hello.me, Ordering::Relaxed);
        self.host.store(host, Ordering::Relaxed);
        // The connection is its own to read from now on, however long it is
        // quiet: the door says when the server has gone. Said to the handle
        // that is read from. On Windows each handle to a socket keeps a
        // timeout of its own, and one that kept the answer's would drop a
        // project every time nobody did anything for that long.
        door.reader.get_ref().set_read_timeout(None).ok()?;
        door.socket.set_write_timeout(Some(PATIENCE)).ok()?;
        let writing = door.socket.try_clone().ok()?;
        let (link, told) = {
            let mut line = self.line.lock();
            let told = line.writable != hello.writable;
            *line = Line {
                link: line.link + 1,
                head: hello.head,
                behind: hello.head,
                heard: hello.heard,
                flight: None,
                rest: Instant::now(),
                writable: hello.writable,
                limit: BATCH_PLAIN,
                piece: line.piece,
                snapshot,
                bytes,
                full: false,
            };
            (line.link, told)
        };
        let me = Person {
            id: hello.me,
            name: self.seat.name.clone(),
            colour: hello.colour,
            presence: Presence::default(),
            host,
            look: self.seat.look(),
        };
        *self.people.lock() = vec![me];
        if let Some(list) = people {
            self.seated(&list);
        }
        let (out, outbox) = sync_channel(OUTBOX);
        *self.link.lock() = Some((out, door.socket.try_clone().ok()?));
        let inner = self.clone();
        let spawned = thread::Builder::new()
            .name("lumit-online-send".into())
            .spawn(move || inner.write(link, writing, &outbox));
        if spawned.is_err() {
            self.hang_up();
            return None;
        }
        if told {
            (self.events)(Event::Writable(hello.writable));
        }
        self.looked(looks);
        (self.events)(Event::Activity);
        Some(door)
    }

    /// A connection's writer: send what is queued, and on every beat what
    /// this end is looking at, if that has changed, and the edits made here,
    /// if there are any to send. It ends when the connection is let go of.
    fn write(&self, link: u64, mut socket: TcpStream, outbox: &Receiver<Out>) {
        let mut send = |frame: Option<Vec<u8>>| match frame {
            Some(frame) => socket.write_all(&frame).is_ok(),
            None => true,
        };
        loop {
            self.let_go();
            let sent = match outbox.recv_timeout(BEAT) {
                Ok(Out::Frame(frame)) => send(Some(frame)),
                Ok(Out::Wake) => send(self.batch(link)),
                Err(RecvTimeoutError::Timeout) => send(self.looking()) && send(self.batch(link)),
                Err(RecvTimeoutError::Disconnected) => false,
            };
            // An append the server never answers is a line that has gone.
            if !sent || self.line.lock().flight.as_ref().is_some_and(Flight::lost) {
                break;
            }
        }
        let _ = socket.shutdown(Shutdown::Both);
    }

    /// What this end is looking at, sealed, if the others have not been
    /// told. Not while a batch too big for the socket is on its way: the
    /// door reads nothing more until it has stored that, and a writer stuck
    /// behind it for long enough would take the line for dead.
    fn looking(&self) -> Option<Vec<u8>> {
        let key = self.seal.get()?;
        if self.line.lock().flight.as_ref().is_some_and(Flight::slow) {
            return None;
        }
        let mut said = self.said.lock();
        let fresh = self.presence.lock().take();
        let presence = match fresh {
            Some(presence) => presence,
            // What this end is changing is said again while it is.
            None => {
                let (presence, at) = said.as_ref()?;
                (at.elapsed() >= SAID_AGAIN).then(|| presence.clone())?
            }
        };
        let again = !presence.changing.is_empty();
        *said = again.then(|| (presence.clone(), Instant::now()));
        let sealed = seal(key, b"presence", &serde_json::to_vec(&presence).ok()?).ok()?;
        // The server passes it on with the seat it came from in front.
        (sealed.len() + 5 < MESSAGE_LIMIT).then(|| frame(PRESENCE, &[&sealed]))
    }

    /// The next append, if there are edits to send and nothing in the way:
    /// every edit made here that the server has not answered, as each sits
    /// on the document now, for the head this machine has.
    ///
    /// The head is read before the edits. If someone else's edit goes in
    /// between the two, the edits are landed on a later head than the one
    /// named, which the server by then has too, and it answers that this is
    /// stale. It can never be the other way round.
    fn batch(&self, link: u64) -> Option<Vec<u8>> {
        // A machine that can only look sends no edit, and the server would
        // take none. What is made on it is taken back here, so what it
        // shows is always the project as everyone has it.
        if self.seat.look() {
            let made = self.store.pending();
            if made.iter().any(|(id, _)| self.store.reject(*id).is_err()) {
                self.hang_up();
            }
            // The person is told, or it looks like a fault. Once for a run
            // of them, which is what dragging something is.
            if !made.is_empty() {
                let last = self.tried.lock().replace(Instant::now());
                if last.is_none_or(|last| last.elapsed() > TRYING) {
                    (self.events)(Event::LookOnly);
                }
            }
            return None;
        }
        let key = self.seal.get()?;
        let (base, limit) = {
            let line = self.line.lock();
            let waiting = line.flight.is_some() || line.head < line.behind || line.full;
            if line.link != link || waiting || Instant::now() < line.rest {
                return None;
            }
            (line.head, line.limit.min(line.piece))
        };
        let mut edits = self.store.pending();
        // An edit that no longer lands, because of what someone else did,
        // is taken back here and never sent.
        let gone: Vec<u64> = edits
            .iter()
            .filter_map(|(id, landed)| landed.is_none().then_some(*id))
            .collect();
        if !gone.is_empty() {
            if gone.iter().any(|id| self.store.reject(*id).is_err()) {
                self.hang_up();
                return None;
            }
            edits = self.store.pending();
        }
        let mut edits: Vec<(u64, Op)> = edits
            .into_iter()
            .map_while(|(id, landed)| Some((id, landed?)))
            .take(u32::MAX as usize)
            .collect();
        // The notes this person has written that the order has not got.
        // They go with the edits, or by themselves when there are none.
        let notes: Vec<Note> = {
            let activity = self.activity.lock();
            let waiting = activity.waiting.iter().take(NOTES_A_BATCH);
            waiting.cloned().collect()
        };
        if edits.is_empty() && notes.is_empty() {
            return None;
        }
        // A batch is its place in the order, and a place is an edit. Notes
        // with no edit to go with take one that changes nothing.
        let nothing = Op::Batch { ops: Vec::new() };
        let (who, at) = (self.store.snapshot().id, activity::now());
        let (packed, comps) = loop {
            let mut ops: Vec<&Op> = edits.iter().map(|(_, op)| op).collect();
            if ops.is_empty() {
                ops.push(&nothing);
            }
            let ops = serde_json::to_value(&ops).ok()?;
            let name = &self.seat.name;
            let (ops, notes) = (&ops, &notes[..]);
            let batch = Batch {
                who,
                name,
                at,
                ops,
                notes,
            };
            let plain = scrubbed(&batch, &self.seat.names).ok()?;
            // Deflated before it is sealed, as a snapshot is. What it comes
            // to then is what crosses the line, so that is what is held to
            // the limits. What it opens to is held to the most a machine
            // will inflate.
            let opens = plain.len() <= BATCH_PLAIN;
            let packed = miniz_oxide::deflate::compress_to_vec(&plain, 6);
            // One edit goes by itself however far batches have been cut
            // down, short of what the server never takes. So do notes.
            let alone = edits.len() <= 1;
            if opens && (packed.len() <= limit || alone) {
                break (packed, activity::comps(ops));
            }
            if let [(id, _)] = edits[..] {
                // Too large for the server ever to take, so it can never be
                // shared, and every edit made after it would wait behind
                // it. It is taken back.
                let _ = self.store.reject(id);
                return None;
            }
            edits.truncate(edits.len() / 2);
        };
        let count = edits.len().max(1) as u32;
        let last = edits.last().map(|(last, _)| *last);
        let sealed = seal(key, &batch_data(base, count), &packed).ok()?;
        let mut line = self.line.lock();
        // Not for a copy that is closing: what is on its way has to be what
        // was noted as on its way.
        if self.stop.load(Ordering::Relaxed) || line.link != link || line.head != base {
            return None;
        }
        // What the server adds the count to for the last edit it has had
        // from this machine. The numbers in between may have gaps. With no
        // edit in it, that is the one it has already.
        let first = last.unwrap_or(line.heard);
        let first = first.wrapping_add(1).wrapping_sub(u64::from(count));
        line.flight = Some(Flight {
            base,
            edits,
            bytes: packed.len(),
            since: Instant::now(),
            at,
            comps,
            count,
            notes: notes.len(),
        });
        let header = [
            &base.to_le_bytes()[..],
            &first.to_le_bytes(),
            &count.to_le_bytes(),
        ];
        Some(frame(APPEND, &[&header.concat(), &sealed]))
    }

    /// The member's thread: read from the server until it is lost, then
    /// look for it again and merge, for as long as the copy is a member.
    fn run(self: Arc<Self>, mut live: Option<Door>) {
        loop {
            if let Some(mut door) = live.take() {
                let people = self.people.lock().clone();
                self.tell_people(people);
                self.in_step();
                let ended = self.listen(&mut door);
                self.hang_up();
                if ended.is_some() || self.stop.load(Ordering::Relaxed) {
                    if !self.stop.load(Ordering::Relaxed) {
                        self.store.unshare();
                    }
                    if let Some(ending) = ended {
                        self.forget();
                        (self.events)(Event::Ended(ending));
                    }
                    return;
                }
            }
            self.adrift();
            self.alone();
            (self.events)(Event::Away);
            self.keep(false);
            // The server's document names an effect's file and not where it
            // is. Where this machine reads each from is noted off its own
            // document, the way sending it would, to be put back on the way
            // in. A copy opened again has sent nothing to have noted them.
            let _ = scrubbed(&*self.store.snapshot(), &self.seat.names);

            let mut wait = 1;
            // Turned away and not cut off, so the first try waits its turn.
            if self.barred.swap(false, Ordering::Relaxed) {
                wait = self.wait(LONGEST_WAIT);
            }
            // The invite the person has been told leads somewhere else, or
            // was replaced while this machine was away.
            let (mut elsewhere, mut replaced) = (None, None);
            live = loop {
                if self.stop.load(Ordering::Relaxed) {
                    return;
                }
                let ending = match self.connect() {
                    // The copy closed while the server was being asked.
                    // What was kept stays kept, for when it is opened again.
                    Ok(_) if self.stop.load(Ordering::Relaxed) => continue,
                    // Another project altogether. There is nothing here to
                    // merge with, so it is said once and the looking goes on.
                    Ok(None) => {
                        let invite = Some(self.seat.invite.lock().clone());
                        if elsewhere != invite {
                            elsewhere = invite;
                            (self.events)(Event::Elsewhere);
                        }
                        wait = self.wait(wait);
                        continue;
                    }
                    Ok(Some((connected, mut document))) => {
                        *self.apart.lock() = None;
                        // The way out before the merge, which sends by it.
                        let heard = connected.hello.heard;
                        let Some(live) = self.link_up(connected) else {
                            wait = self.wait(wait);
                            continue;
                        };
                        let have = self.store.snapshot();
                        settle(&mut document, Some(&have), self.seat.root.as_deref());
                        // What this machine's own file packs is its own too,
                        // and for the host that file is the project's first.
                        // Without this its next save would leave out footage
                        // the file may hold the only copy of.
                        document.packed.clone_from(&have.packed);
                        self.keep(false);
                        let (held, refused, before) = {
                            // Held across the merge, so that a copy closing
                            // this moment keeps either what was there before
                            // it or all of what came of it.
                            let mut kept = self.kept.lock();
                            let (mut conflicts, mut refused, before) =
                                self.store.rejoin(document, heard);
                            // Nothing made on a machine that can only look
                            // goes in, so there is nothing to choose between
                            // and nothing lost.
                            if self.seat.look() {
                                conflicts.clear();
                                refused.clear();
                            }
                            let went = !conflicts.is_empty() || !refused.is_empty();
                            let held = {
                                let mut all = self.conflicts.lock();
                                all.extend(conflicts);
                                all.len()
                            };
                            // Not if the copy closed meanwhile: then nothing
                            // of the merge was sent, and what is kept is
                            // what the copy opened again merges from.
                            if self.stop.load(Ordering::Relaxed) {
                                return;
                            }
                            if let Some(kept) = kept.take() {
                                kept.remove();
                            }
                            self.doubt.lock().clear();
                            (held, refused, went.then_some(before))
                        };
                        // The person's way back from a return that held or
                        // dropped something of theirs, kept before they are
                        // told there is anything to come back from.
                        let mut dropped = Vec::new();
                        if let Some(before) = &before {
                            dropped = crate::local::dropped(&refused, before);
                            kept::keep_before(before, refused.len(), &dropped);
                        }
                        let refused = refused.len();
                        (self.events)(Event::Back {
                            held,
                            refused,
                            dropped,
                        });
                        break Some(live);
                    }
                    // Another version of Lumit keeps it. That passes: this
                    // one is updated, or the project is moved forward. So
                    // it is said when it is news, the copy stays a member
                    // with what it has kept, and the looking goes on.
                    Err(ShareError::Versions(apart)) => {
                        let news = Some(apart.clone());
                        let told = std::mem::replace(&mut *self.apart.lock(), news);
                        if told.as_ref() != Some(&apart) {
                            (self.events)(Event::Versions(apart));
                        }
                        wait = self.wait(wait);
                        continue;
                    }
                    Err(ShareError::Refused(refusal)) => Ending::Refused(refusal),
                    Err(ShareError::Unsafe) => Ending::Unsafe,
                    // What became of the project while this machine was
                    // away, by the server's word. None of it passes, so it
                    // ends here as it would have had the machine been there
                    // to be told.
                    Err(ShareError::Server(code)) if code == "closed" => Ending::Closed,
                    Err(ShareError::Server(code)) if code == "removed" || code == "look_off" => {
                        Ending::Removed
                    }
                    // Replaced, and this is the machine that was removed
                    // with it.
                    Err(ShareError::Server(code)) if code == "moved" && self.removed() => {
                        Ending::Removed
                    }
                    // The invite was replaced, and this machine was not
                    // there to be sent the new one. Nothing answers by the
                    // old one again, so it is said once and nothing more is
                    // asked until a new invite is given. The copy stays a
                    // member, with what it has kept.
                    Err(ShareError::Server(code)) if code == "moved" => {
                        let invite = Some(self.seat.invite.lock().clone());
                        if replaced != invite {
                            replaced = invite;
                            (self.events)(Event::Replaced);
                        }
                        while !self.stop.load(Ordering::Relaxed) {
                            if self.wait(LONGEST_WAIT) == 1 {
                                break;
                            }
                        }
                        continue;
                    }
                    Err(_) => {
                        wait = self.wait(wait);
                        continue;
                    }
                };
                if self.stop.load(Ordering::Relaxed) {
                    return;
                }
                self.store.unshare();
                // Not kept past here. The copy is edited on with nothing
                // writing those edits down, so what was kept would be an
                // older copy by the time it was opened again.
                self.forget();
                (self.events)(Event::Ended(ending));
                return;
            };
        }
    }

    /// Find the project and catch up with it. `None` when the invite leads
    /// to another project. Not while another version of Lumit keeps it, and
    /// not while the server is taking no edits: nothing kept here could go
    /// in, so it is asked again later and nothing is fetched.
    fn connect(&self) -> Result<Option<(Connected, Document)>, ShareError> {
        let met = find(&self.seat)?;
        if project_of(&met.seal) != self.project {
            return Ok(None);
        }
        if let Some(apart) = met.apart.clone() {
            // Whose machine this is shows before it is in, since that is
            // what decides who may move the project forward.
            self.host.store(met.host, Ordering::Relaxed);
            return Err(ShareError::Versions(apart));
        }
        if !met.writable {
            self.unwritable();
            return Err(ShareError::Unreachable);
        }
        // Turned away for room, with the project too large to store a
        // snapshot of. Nothing goes in until someone stores one, and
        // whoever does has a batch to send after it. So while the head is
        // where it was there is nothing to come back to, and what was made
        // here stays kept on disk. Once it has moved, a snapshot fitted.
        let moved = {
            let mut walled = self.walled.lock();
            if walled.is_some() && *walled == met.head {
                return Err(ShareError::Unreachable);
            }
            walled.take().is_some()
        };
        if moved {
            self.outgrown(false);
        }
        let had = self.had(met.snapshot);
        let carried = had.is_some();
        let fetched = fetch(met, &self.seat, &self.stop, had);
        // Carrying on from what this machine had did not work, and not for
        // the line: the server no longer has the order from there, or an
        // entry did not apply. The next try fetches everything.
        let line = |e: &ShareError| matches!(e, ShareError::Unreachable | ShareError::Io(_));
        if carried && fetched.as_ref().is_err_and(|e| !line(e)) {
            self.whole.store(true, Ordering::Relaxed);
            return Err(ShareError::Unreachable);
        }
        fetched.map(Some)
    }

    /// What this machine still has of the project, to carry on from when it
    /// comes back: the document as it stood at the last sequence it reached,
    /// with nothing made here since on it, and the lists as they stood. So
    /// a return fetches what came after, and not the project again.
    ///
    /// `None` when everything has to be fetched: this machine has not been
    /// in the project since it was opened, so nothing says where in the
    /// order what it holds stands, or the server's snapshot has passed it
    /// and the entries between are gone, or the last try from here failed.
    fn had(&self, snapshot: Option<u64>) -> Option<Had> {
        if self.whole.swap(false, Ordering::Relaxed) {
            return None;
        }
        let line = self.line.lock();
        if line.link == 0 || snapshot? > line.head {
            return None;
        }
        let (base, _) = self.store.apart(0)?;
        let activity = self.activity.lock();
        Some(Had {
            head: line.head,
            document: Document::clone(&base),
            activity: activity.list.clone(),
            notes: activity.notes.clone(),
            snapshot: line.snapshot,
            bytes: line.bytes,
        })
    }

    /// Whether this copy is the machine the server removed when the invite
    /// was replaced. The server knows that machine by its token, and a copy
    /// closed and opened again goes by a new one. So one that has not been
    /// seated since asks once more, by the token it went by before.
    fn removed(&self) -> bool {
        let seated = self.me.load(Ordering::Relaxed) != 0;
        let Some(was) = self.seat.was.filter(|_| !seated) else {
            return false;
        };
        let asked = dial_as(&self.seat, was).and_then(|mut door| door.ask(META, &[], META_IS));
        matches!(asked, Err(ShareError::Server(code)) if code == "removed")
    }

    /// Whether a snapshot of the project is too large to store, as this
    /// machine last found. Said when it is news.
    fn outgrown(&self, too_large: bool) {
        if self.too_large.swap(too_large, Ordering::Relaxed) != too_large {
            (self.events)(Event::TooLarge(too_large));
        }
    }

    /// Whether the invite that can only look is on, by the server's word.
    /// Said when it is news.
    fn looked(&self, on: bool) {
        if self.looks.swap(on, Ordering::Relaxed) != on {
            (self.events)(Event::LookInvite(on));
        }
    }

    /// The server is taking no edits. Said once, when it is news.
    fn unwritable(&self) {
        let told = std::mem::replace(&mut self.line.lock().writable, false);
        if told {
            (self.events)(Event::Writable(false));
        }
    }

    /// Wait `seconds` before looking for the server again, and answer how
    /// long to wait next time. In short steps, as a guest waits, and cut
    /// short when the interface opens a door there was none of.
    fn wait(&self, seconds: u64) -> u64 {
        let door = || ONLINE_DOOR.load(Ordering::Relaxed) != 0;
        let shut = !door();
        for _ in 0..seconds * 10 {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            self.keep(false);
            if self.hurry.swap(false, Ordering::Relaxed) || (shut && door()) {
                return 1;
            }
            thread::sleep(Duration::from_millis(100));
        }
        (seconds * 2).min(LONGEST_WAIT)
    }

    /// Look for the project by `invite` from now on, and dial again at once
    /// if that is somewhere new. What is kept names the old invite, and is
    /// written again with this.
    fn adopt(&self, invite: Invite) {
        let was = self.seat.channel();
        *self.seat.invite.lock() = invite;
        if let Some(kept) = self.kept.lock().as_mut() {
            kept.stale = true;
        }
        self.hurry.store(true, Ordering::Relaxed);
        if was != self.seat.channel() {
            self.hang_up();
        }
    }

    /// Read from the server until the connection goes or this machine is
    /// found to be out of step with it, either of which is put right by
    /// finding it again. An ending when the server said this is over.
    fn listen(self: &Arc<Self>, door: &mut Door) -> Option<Ending> {
        let key = self.seal.get()?;
        while !self.stop.load(Ordering::Relaxed) {
            let Ok((kind, bytes)) = door.recv() else {
                break;
            };
            let kept = match kind {
                ENTRY => self.entered(key, &bytes),
                ACK => self.answered(&bytes),
                STALE => self.stale(&bytes),
                REFUSED => self.refused(&bytes),
                // The door could not store a batch too big for the socket,
                // so it was never sent.
                FAILED => self.unsent(&bytes),
                PEOPLE => self.seated(&bytes).map(|people| self.tell_people(people)),
                PRESENT => self.seen(key, &bytes),
                NOTED => {
                    let mut bytes = &bytes[..];
                    let from = number(&mut bytes).map(u32::from_le_bytes);
                    let body = open(key, b"note", bytes);
                    let body = body.and_then(|body| serde_json::from_slice(&body).ok());
                    if let Some((from, body)) = from.zip(body) {
                        (self.events)(Event::Note { from, body });
                    }
                    Some(())
                }
                MOVED => {
                    // The new invite of this machine's own kind. The one
                    // that edits is sealed with what only those who can
                    // edit hold, and the other as everything else is.
                    let look = self.seat.look();
                    let opens = self.seat.keys().told.unwrap_or(*key);
                    let invite = open(&opens, b"invite", &bytes);
                    let invite = invite.and_then(|text| String::from_utf8(text).ok());
                    let invite = invite.and_then(|text| text.parse::<Invite>().ok());
                    // Dialled again at once, which is what ends this
                    // connection. Not if it is where this machine looks
                    // already: the host's own, which keeps the invite it
                    // hands out and not this one that needs no password.
                    let news = |invite: &Invite| {
                        let here = invite.key == self.seat.channel();
                        invite.is_online() && !invite.locked && invite.look == look && !here
                    };
                    if let Some(invite) = invite.filter(news) {
                        self.adopt(invite);
                    }
                    Some(())
                }
                LOOKS => {
                    self.looked(bytes == [1]);
                    Some(())
                }
                CLOSED => return Some(Ending::Closed),
                REMOVED => return Some(Ending::Removed),
                _ => None,
            };
            if kept.is_none() {
                break;
            }
        }
        None
    }

    /// Someone else's batch: apply its edits as they are, under the ones
    /// made here that the server has not answered. `None` when it does not
    /// follow on or does not apply, which is this machine out of step.
    fn entered(&self, key: &Key, bytes: &[u8]) -> Option<()> {
        let head = self.line.lock().head;
        let entered = entry(key, &self.seat, bytes, head).ok()?;
        let Entry {
            last,
            peer,
            ops,
            by,
            notes,
        } = entered;
        let (edits, noted) = (counted(&ops), !notes.is_empty());
        // Said before it is applied, so nothing is sent in between that the
        // server would only call stale.
        {
            let mut line = self.line.lock();
            line.behind = line.behind.max(last);
        }
        let root = self.seat.root.as_deref();
        for (nth, mut op) in ops.into_iter().enumerate() {
            place(&mut op, &self.store.snapshot(), root);
            let id = head.saturating_add(1 + nth as u64);
            let tag = RemoteTag { peer, id };
            self.store.commit_remote(&op, None, tag).ok()?;
        }
        // Into the list before the head moves on, so that whoever finds the
        // head there finds the list there too.
        {
            let mut activity = self.activity.lock();
            if edits > 0 {
                activity.add(by, edits, last);
            }
            activity.noted(notes, last);
        }
        {
            let mut line = self.line.lock();
            line.head = last;
            line.bytes += bytes.len() as u64;
        }
        if edits > 0 || noted {
            (self.events)(Event::Activity);
        }
        self.wake();
        Some(())
    }

    /// The batch on its way went in, as it was sent: nothing can have gone
    /// in under it, or it would have been stale. Its edits are answered, and
    /// if the head has run far enough past the stored snapshot a new one is
    /// stored. `None` when that is not how things stand here.
    fn answered(self: &Arc<Self>, mut bytes: &[u8]) -> Option<()> {
        let head = u64::from_le_bytes(number(&mut bytes)?);
        let (told, due) = {
            let mut line = self.line.lock();
            // Left where it is until its edits are answered. If this gives
            // up part of the way, the batch is still what is in doubt.
            {
                let flight = line.flight.as_ref()?;
                if line.head.checked_add(u64::from(flight.count)) != Some(head) {
                    return None;
                }
                // Each is there as it was sent, so answering it only takes
                // it off the list: nothing is told while this lock is held.
                let lands = self.store.pending();
                let same = |((id, op), (had, landed)): (&(u64, Op), &(u64, Option<Op>))| {
                    id == had && landed.as_ref() == Some(op)
                };
                let all = flight.edits.iter().zip(&lands).all(same);
                if lands.len() < flight.edits.len() || !all {
                    return None;
                }
                for (id, op) in &flight.edits {
                    self.store.commit_answer(*id, op).ok()?;
                }
            }
            let flight = line.flight.take()?;
            // Into this machine's own list as the batch said of itself, so
            // that it is the list everyone else comes to. And before the
            // head moves on, so that whoever finds the head there finds the
            // list there too.
            let by = By {
                who: self.store.snapshot().id,
                name: self.seat.name.clone(),
                at: flight.at,
                comps: flight.comps,
            };
            let edits = flight.edits.len();
            if let Some((last, _)) = flight.edits.last() {
                line.heard = *last;
            }
            {
                let mut activity = self.activity.lock();
                if edits > 0 {
                    activity.add(by, edits, head);
                }
                // The notes that went with it are everyone's from here.
                activity.sent(flight.notes, head);
            }
            line.head = head;
            line.behind = line.behind.max(head);
            line.bytes += flight.bytes as u64;
            line.piece = line.piece.saturating_mul(2).min(BATCH_PLAIN);
            line.rest = Instant::now() + BEAT;
            let told = !std::mem::replace(&mut line.writable, true);
            let past = line.head.saturating_sub(line.snapshot);
            (told, past > COMPACT_EDITS || line.bytes > COMPACT_BYTES)
        };
        (self.events)(Event::Activity);
        if told {
            (self.events)(Event::Writable(true));
        }
        if due {
            self.compact(head);
        }
        self.wake();
        Some(())
    }

    /// The batch on its way did not go in: the server's head had moved on
    /// to the one it names. The entries this machine had not heard are on
    /// their way or here, and once they are in the edits are sent again as
    /// they sit then.
    fn stale(&self, mut bytes: &[u8]) -> Option<()> {
        let head = u64::from_le_bytes(number(&mut bytes)?);
        {
            let mut line = self.line.lock();
            // A batch that takes long to cross this machine's line misses
            // its place for as long as anyone else is at work: by the time
            // it is there the head has moved on, and all of it goes up
            // again. So the next is half as much, which is there sooner.
            // Each that goes in lets the one after be twice as much.
            let flight = line.flight.take();
            // Stale on the head it was made on: nothing was missed, and it
            // is another machine's turn. The batch is not cut down for
            // that. It goes again after a short rest.
            let waiting = flight.as_ref().is_some_and(|flight| flight.base == head);
            let big = |flight: &Flight| !waiting && flight.bytes > SMALL_BATCH;
            if let Some(flight) = flight.filter(big) {
                line.piece = (flight.bytes / 2).max(SMALL_BATCH);
            }
            line.behind = line.behind.max(head);
            line.rest = Instant::now();
            if waiting {
                line.rest += TURN_REST;
            }
        }
        self.wake();
        Some(())
    }

    /// The server would not take the batch on its way. Its edits stay where
    /// they are, unanswered.
    fn refused(self: &Arc<Self>, bytes: &[u8]) -> Option<()> {
        let mut line = self.line.lock();
        match bytes {
            // The host's account no longer has Lumit Pro, which can last. So
            // the line is given up, as if the server had gone: from there
            // the edits made meanwhile are kept on disk, and the server is
            // asked every so often whether it takes them again.
            [2] => {
                line.flight = None;
                drop(line);
                self.unwritable();
                self.barred.store(true, Ordering::Relaxed);
                return None;
            }
            // Too large. Sent again in smaller batches, down to one edit.
            // One the server will not take even by itself can never be
            // shared, and is taken back.
            [3] => {
                let flight = line.flight.take()?;
                line.limit = (flight.bytes / 2).max(1);
                drop(line);
                if let [(id, _)] = flight.edits[..] {
                    self.store.reject(id).ok()?;
                }
                self.wake();
            }
            // The server keeps no more of the order past its snapshot until
            // a newer one is stored. It says so only to a machine at the
            // head, so this one stores it, of the project as the order has
            // it, and the batch is sent again once it is in. The edits wait
            // where they are meanwhile, as they do for any answer, and no
            // other machine is waited on. If none can be stored the line is
            // given up, as when the server takes no edits, so that the
            // edits are kept on disk while they wait.
            [5] => {
                line.flight = None;
                line.full = true;
                let head = line.head;
                drop(line);
                if !self.compact(head) {
                    self.barred.store(true, Ordering::Relaxed);
                    return None;
                }
            }
            _ => return None,
        }
        Some(())
    }

    /// The batch on its way never left the door, which could not store it.
    /// The line is given up, so that the edits are kept on disk while they
    /// wait to be sent again, and the next batch is half as much. When it
    /// was the server that would not have it there is no hurry back, as
    /// when the server takes no edits. When the line went under it, this
    /// machine comes back as it does from any drop.
    fn unsent(&self, why: &[u8]) -> Option<()> {
        let mut line = self.line.lock();
        if let Some(flight) = line.flight.take() {
            line.piece = (flight.bytes / 2).max(SMALL_BATCH);
        }
        if !matches!(failed(why), ShareError::Unreachable) {
            self.barred.store(true, Ordering::Relaxed);
        }
        None
    }

    /// Someone is looking at something else.
    fn seen(&self, key: &Key, mut bytes: &[u8]) -> Option<()> {
        let peer = u32::from_le_bytes(number(&mut bytes)?);
        let presence = open(key, b"presence", bytes);
        let presence = presence.and_then(|p| serde_json::from_slice::<Presence>(&p).ok());
        // One that does not read is passed over. Nothing depends on it.
        let Some(presence) = presence else {
            return Some(());
        };
        let people = {
            let mut people = self.people.lock();
            if let Some(person) = people.iter_mut().find(|p| p.id == peer) {
                person.presence = presence.tidied();
                let mut changing = self.changing.lock();
                changing.retain(|(who, _)| *who != peer);
                if !person.presence.changing.is_empty() {
                    changing.push((peer, Instant::now()));
                }
            }
            people.clone()
        };
        self.tell_people(people);
        Some(())
    }

    /// Let go of what anyone is changing who has not said so for [`HELD`]:
    /// their machine may have vanished, and then nothing is held for it.
    fn let_go(&self) {
        let gone: Vec<u32> = {
            let mut changing = self.changing.lock();
            let gone = changing.iter().filter(|(_, at)| at.elapsed() >= HELD);
            let gone: Vec<u32> = gone.map(|(who, _)| *who).collect();
            changing.retain(|(who, _)| !gone.contains(who));
            gone
        };
        if gone.is_empty() {
            return;
        }
        let people = {
            let mut people = self.people.lock();
            for person in people.iter_mut().filter(|p| gone.contains(&p.id)) {
                person.presence.changing.clear();
            }
            people.clone()
        };
        self.tell_people(people);
    }

    /// Store a new snapshot, of the document at the head `seq`, from a
    /// thread of its own. That is the document here without the edits made
    /// here that the order has not got, so one is stored whatever this
    /// machine is in the middle of. Called from the thread that takes
    /// batches in, which is what makes that document the one at `seq`. Not
    /// for a copy that is closing, whose store may be nobody's guest by
    /// now. Whatever the server makes of it, the count starts again from
    /// here: a snapshot it will not take is one somebody else has stored a
    /// newer of. Once it is stored, or the server has one as new, a batch
    /// that was waiting for it is sent. One too large to store is said
    /// ([`Event::TooLarge`]), and a batch that was waiting for it has
    /// nowhere to go: the line is given up, so that what was made here is
    /// kept on disk. False when none is on its way.
    fn compact(self: &Arc<Self>, seq: u64) -> bool {
        if self.packing.swap(true, Ordering::Relaxed) {
            return true;
        }
        // Read before `stop`, which is set ahead of the store being let go
        // of.
        let document = self.store.heard();
        let document = document.filter(|_| !self.stop.load(Ordering::Relaxed));
        // Who changed what up to here goes with it. This is the thread that
        // takes batches in, so the list is the one for this sequence.
        let (list, notes) = {
            let activity = self.activity.lock();
            (activity.list.clone(), activity.notes.clone())
        };
        let spawned = document.map(|document| {
            let inner = self.clone();
            thread::Builder::new()
                .name("lumit-online-pack".into())
                .spawn(move || {
                    let stored = inner.seal.get().map(|key| {
                        let lists = (&list[..], &notes[..]);
                        let sealed = sealed_snapshot(key, &inner.seat, &document, lists, seq)?;
                        let door = &mut dial(&inner.seat)?;
                        put_snapshot(door, seq, &sealed, None, &inner.stop)
                    });
                    let (room, large) = match &stored {
                        Some(Ok(())) => (true, false),
                        // Over this machine's limit, or the server's.
                        Some(Err(ShareError::TooLarge { .. })) => (false, true),
                        Some(Err(ShareError::Server(code))) => {
                            (code == "stale", code == "too_large")
                        }
                        _ => (false, false),
                    };
                    // In this order. A batch turned away after this finds
                    // nothing on its way and starts a snapshot of its own,
                    // and one turned away before it is seen to here.
                    inner.packing.store(false, Ordering::Relaxed);
                    let full = std::mem::take(&mut inner.line.lock().full);
                    if room || large {
                        inner.outgrown(large);
                    }
                    if room {
                        inner.wake();
                    } else if full {
                        if large {
                            *inner.walled.lock() = Some(seq);
                            inner.unwritable();
                        }
                        inner.barred.store(true, Ordering::Relaxed);
                        inner.hang_up();
                    }
                })
        });
        let spawned = matches!(spawned, Some(Ok(_)));
        if spawned {
            let mut line = self.line.lock();
            (line.snapshot, line.bytes) = (seq, 0);
        } else {
            self.packing.store(false, Ordering::Relaxed);
        }
        spawned
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::invite::from_letters;
    use crate::set_online_door;
    use lumit_core::model::{Composition, LinearColour, ProjectItem, SolidDef};
    use lumit_core::shared::footprint;
    use std::collections::{HashMap, HashSet};
    use std::io::BufRead;
    use std::net::TcpListener;

    /// What the door says when the server's socket has gone, and the code
    /// the server closes one with when its project moves to another Lumit.
    const MOVED_ON: u16 = 4426;

    /// A Lumit from before this one, for a machine to run.
    const OLD: &str = "0.0.1";

    /// Someone at the stand-in's socket.
    struct Seated {
        seat: u32,
        colour: u8,
        token: u64,
        /// What the server marks this person with for everyone: 1 signed in
        /// to the account that keeps the project, 2 here by the invite
        /// that can only look.
        mark: u8,
        name: Vec<u8>,
        socket: TcpStream,
    }

    /// One project as the stand-in keeps it, which is what Lumit's server
    /// would: nothing it can read.
    #[derive(Default)]
    struct Stored {
        access: String,
        /// What it was made with, which is most of its meta.
        made: serde_json::Value,
        snapshot: Option<(u64, Vec<u8>)>,
        /// Each batch after the snapshot: its first sequence, how many
        /// edits are in it, and the sealed bytes.
        entries: Vec<(u64, u32, Vec<u8>)>,
        head: u64,
        heard: HashMap<u64, u64>,
        seats: Vec<Seated>,
        next: u32,
        /// Its host's Lumit Pro has ended: it is read and not added to.
        frozen: bool,
        /// How many sealed bytes of entries it keeps past its snapshot
        /// before it turns an append away until a newer one is stored, and
        /// how many it has turned away for that.
        room: Option<usize>,
        turned: u32,
        /// No snapshot fits: storing one is answered as too large.
        tight: bool,
        /// Whose turn it is to append, as the server keeps it: the seats
        /// told an append was stale, first told first, and when the first
        /// of them has until.
        turns: Vec<u32>,
        until: Option<Instant>,
    }

    /// How long the stand-in keeps a turn for whoever it told was stale.
    /// Lumit's server keeps one for three times as long.
    const TURN: Duration = Duration::from_secs(1);

    impl Stored {
        /// A seat has had its turn to append, or has gone. If the turn was
        /// its, the next one's starts now.
        fn took(&mut self, seat: u32) {
            if let Some(at) = self.turns.iter().position(|turn| *turn == seat) {
                self.turns.remove(at);
                if at == 0 {
                    self.until = Some(Instant::now() + TURN);
                }
            }
        }

        /// Whether `access` is what the invite that can only look shows.
        fn looks(&self, access: &str) -> bool {
            self.made["look"] == access
        }

        /// Say something to everyone but the seat `but`.
        fn tell(&self, but: u32, kind: u8, parts: &[&[u8]]) {
            let bytes = frame(kind, parts);
            for seated in self.seats.iter().filter(|seated| seated.seat != but) {
                let _ = (&seated.socket).write_all(&bytes);
            }
        }

        fn people(&self) {
            let mut list = (self.seats.len() as u32).to_le_bytes().to_vec();
            for seated in &self.seats {
                list.extend_from_slice(&seated.seat.to_le_bytes());
                list.extend_from_slice(&[seated.colour, seated.mark]);
                list.extend_from_slice(&(seated.name.len() as u16).to_le_bytes());
                list.extend_from_slice(&seated.name);
            }
            self.tell(0, PEOPLE, &[&list]);
        }

        /// Seat a machine that has gone live: its hello, every entry after
        /// the one it has, and then it is one of the people. One that comes
        /// back takes its seat from the connection it had, which is closed
        /// first.
        fn sit(
            &mut self,
            (token, mark): (u64, u8),
            name: &[u8],
            socket: &TcpStream,
            after: u64,
        ) -> Option<u32> {
            self.seats.retain(|seated| {
                if seated.token == token {
                    let _ = seated.socket.shutdown(Shutdown::Both);
                }
                seated.token != token
            });
            if self.snapshot.as_ref().is_none_or(|(seq, _)| after < *seq) {
                return None;
            }
            self.next += 1;
            let seat = self.next;
            let colour = (1..=u8::MAX).find(|c| self.seats.iter().all(|s| s.colour != *c))?;
            let heard = self.heard.get(&token).copied().unwrap_or(0);
            let (head, heard) = (self.head.to_le_bytes(), heard.to_le_bytes());
            let writable = [u8::from(!self.frozen)];
            let hello = [&seat.to_le_bytes()[..], &[colour], &head, &heard, &writable];
            let mut said = frame(HELLO, &hello);
            for (first, count, sealed) in self.entries.iter().filter(|entry| entry.0 > after) {
                let header = [&first.to_le_bytes()[..], &count.to_le_bytes(), &[0xff; 4]];
                said.extend(frame(ENTRY, &[&header.concat(), sealed]));
            }
            (&*socket).write_all(&said).ok()?;
            self.seats.push(Seated {
                seat,
                colour,
                token,
                mark,
                name: name.to_vec(),
                socket: socket.try_clone().ok()?,
            });
            self.people();
            Some(seat)
        }
    }

    /// What a test has the stand-in do to one machine, which it knows by
    /// its token.
    #[derive(Default)]
    struct Tricks {
        /// There is no way to the server: its connection is dropped and
        /// every call it makes goes unanswered.
        shut: HashSet<u64>,
        /// Its appends go in and are never answered.
        deaf: HashSet<u64>,
        /// Its appends never arrive, and are counted.
        mute: HashMap<u64, u32>,
        /// How many times in a row it has been told an append was stale,
        /// and the most that has come to.
        stale: HashMap<u64, (u32, u32)>,
        /// Not signed in to the account that keeps the project. Everyone
        /// else is taken for its host.
        guests: HashSet<u64>,
        /// On a slow line: how long each kilobyte of an append takes to
        /// arrive from it. Whatever the others append meanwhile is in first.
        slow: HashMap<u64, Duration>,
        /// How many times it has fetched the snapshot.
        fetched: HashMap<u64, u32>,
        /// Far from the server: how long each of its appends is in coming.
        far: HashMap<u64, Duration>,
        /// What the server says became of whatever an access opened, to
        /// whoever asks by it.
        gone: HashMap<String, &'static str>,
        /// The machines removed when an invite was replaced. Each is told
        /// so, where anyone else who asks by that invite is told what
        /// became of it.
        removed: HashSet<u64>,
        /// Its next fetch of the snapshot starts with part of one that a
        /// newer has replaced, and is told so.
        again: HashSet<u64>,
        /// Its next go at going live finds the line gone.
        lost: HashSet<u64>,
        /// Its line drops under an append of more than this many bytes,
        /// which never arrives.
        cut: HashMap<u64, usize>,
    }

    /// The door and Lumit's server in one, in memory: it speaks the door's
    /// frames, keeps each project's snapshot, entries and `heard`, appends
    /// only on the head it is told, and passes on what one machine says to
    /// the others. One for every test here, since there is one door.
    #[derive(Default)]
    struct Stand {
        projects: Mutex<HashMap<String, Stored>>,
        tricks: Mutex<Tricks>,
    }

    fn stand() -> &'static Stand {
        static STAND: OnceLock<&'static Stand> = OnceLock::new();
        STAND.get_or_init(|| {
            let stand: &'static Stand = Box::leak(Box::default());
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            set_online_door(Some(listener.local_addr().unwrap().port()));
            thread::spawn(move || {
                for socket in listener.incoming().flatten() {
                    thread::spawn(move || stand.serve(&socket));
                }
            });
            stand
        })
    }

    fn read(from: &mut impl Read) -> Option<(u8, Vec<u8>)> {
        let mut head = [0u8; 5];
        from.read_exact(&mut head).ok()?;
        let [kind, a, b, c, d] = head;
        let mut bytes = vec![0; u32::from_le_bytes([a, b, c, d]) as usize];
        from.read_exact(&mut bytes).ok()?;
        Some((kind, bytes))
    }

    impl Stand {
        /// Do something with the project `id`, for whoever shows its access.
        fn with<T>(&self, id: &str, access: &str, f: impl FnOnce(&mut Stored) -> T) -> Option<T> {
            let mut projects = self.projects.lock();
            let stored = projects.get_mut(id);
            let opens = |stored: &&mut Stored| stored.access == access || stored.looks(access);
            stored.filter(opens).map(f)
        }

        /// One connection from the engine, until it goes.
        fn serve(&self, socket: &TcpStream) -> Option<()> {
            let mut reader = BufReader::new(socket.try_clone().ok()?);
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;
            let words: Vec<&str> = line.split_whitespace().collect();
            let [CALL, "1", id, access, token, name] = words[..] else {
                return None;
            };
            let (token, name) = (token.parse::<u64>().ok()?, from_letters(name)?);
            if self.tricks.lock().shut.contains(&token) {
                return None;
            }
            let say = |kind: u8, parts: &[&[u8]]| {
                let mut socket = socket;
                socket.write_all(&frame(kind, parts)).ok()
            };
            let fail = |code: &str| {
                let said = serde_json::json!({ "error": code }).to_string();
                say(FAILED, &[said.as_bytes()])
            };
            loop {
                let (kind, bytes) = read(&mut reader)?;
                match kind {
                    CREATE => {
                        let stored = Stored {
                            access: access.to_owned(),
                            made: serde_json::from_slice(&bytes).ok()?,
                            ..Stored::default()
                        };
                        self.projects.lock().insert(id.to_owned(), stored);
                        say(OK, &[])?;
                    }
                    META => {
                        let why = {
                            let tricks = self.tricks.lock();
                            let why = tricks.gone.get(access).copied();
                            why.map(|why| match tricks.removed.contains(&token) {
                                true => "removed",
                                false => why,
                            })
                        };
                        if let Some(why) = why {
                            fail(why)?;
                            continue;
                        }
                        let meta = self.with(id, access, |stored| {
                            let mut meta = stored.made.clone();
                            if stored.looks(access) {
                                meta["wrap"] = meta["look_wrap"].take();
                            }
                            meta["looks"] = (!stored.made["look"].is_null()).into();
                            let snapshot = stored.snapshot.as_ref().map(|(seq, _)| *seq);
                            meta["snapshot"] = snapshot.into();
                            meta["head"] = stored.head.into();
                            meta["host"] = (!self.tricks.lock().guests.contains(&token)).into();
                            meta["writable"] = (!stored.frozen).into();
                            meta.to_string()
                        });
                        match meta {
                            Some(meta) => say(META_IS, &[meta.as_bytes()])?,
                            None => fail("not_found")?,
                        }
                    }
                    GET_SNAPSHOT => {
                        *self.tricks.lock().fetched.entry(token).or_default() += 1;
                        let stored = self.with(id, access, |stored| stored.snapshot.clone());
                        match stored.flatten() {
                            Some((seq, sealed)) => {
                                if self.tricks.lock().again.remove(&token) {
                                    say(PART_IS, &[b"the start of another"])?;
                                    say(PART_IS, &[])?;
                                    say(AGAIN_IS, &[])?;
                                }
                                for part in sealed.chunks(PIECE) {
                                    say(PART_IS, &[part])?;
                                }
                                say(END_IS, &[&seq.to_le_bytes()])?;
                            }
                            None => fail("not_found")?,
                        }
                    }
                    PUT_SNAPSHOT => {
                        let mut bytes = &bytes[..];
                        let seq = u64::from_le_bytes(number(&mut bytes)?);
                        let size = u64::from_le_bytes(number(&mut bytes)?);
                        // What is left says which Lumit takes the project
                        // over. Only the host may say another, and only of
                        // the head: every edit of the older Lumit's has to
                        // be in what the newer one stores.
                        let to: Option<Value> = serde_json::from_slice(bytes).ok();
                        let mut sealed = Vec::new();
                        loop {
                            match read(&mut reader)? {
                                (PART, part) => sealed.extend(part),
                                (END, _) => break,
                                _ => return None,
                            }
                        }
                        let guest = self.tricks.lock().guests.contains(&token);
                        let taken = self.with(id, access, |stored| {
                            if stored.looks(access) {
                                return "look_only";
                            }
                            let to = to.filter(|to| {
                                let differs = |field| to[field] != stored.made[field];
                                differs("version") || differs("schema")
                            });
                            if to.is_some() && guest {
                                return "forbidden";
                            }
                            if stored.tight {
                                return "too_large";
                            }
                            let moving = to.is_some();
                            let past = |had: u64| seq > had || (seq == had && moving);
                            let newer = stored.snapshot.as_ref().is_none_or(|(had, _)| past(*had));
                            let behind = seq > stored.head || (moving && seq != stored.head);
                            if behind || !newer || sealed.len() as u64 != size {
                                return "stale";
                            }
                            let ends =
                                |entry: &(u64, u32, Vec<u8>)| entry.0 + u64::from(entry.1) - 1;
                            stored.entries.retain(|entry| ends(entry) > seq);
                            stored.snapshot = Some((seq, sealed));
                            if let Some(to) = to {
                                stored.made["version"] = to["version"].clone();
                                stored.made["schema"] = to["schema"].clone();
                                stored.tell(0, LOST, &[&MOVED_ON.to_le_bytes()]);
                                for seated in stored.seats.drain(..) {
                                    let _ = seated.socket.shutdown(Shutdown::Both);
                                }
                            }
                            ""
                        });
                        match taken {
                            Some("") => say(OK, &[])?,
                            Some(code) => fail(code)?,
                            None => fail("stale")?,
                        }
                    }
                    LIVE => {
                        if self.tricks.lock().lost.remove(&token) {
                            return None;
                        }
                        let after = u64::from_le_bytes(number(&mut &bytes[..])?);
                        let host = !self.tricks.lock().guests.contains(&token);
                        let seat = self.with(id, access, |stored| {
                            let mark = u8::from(host) | (u8::from(stored.looks(access)) << 1);
                            stored.sit((token, mark), &name, socket, after)
                        });
                        match seat.flatten() {
                            Some(seat) => return self.live(id, seat, token, &mut reader),
                            None => fail("not_found")?,
                        }
                    }
                    _ => return None,
                }
            }
        }

        /// A machine that is live, until it goes or is no longer seated.
        fn live(&self, id: &str, seat: u32, token: u64, reader: &mut impl Read) -> Option<()> {
            while let Some((kind, bytes)) = read(reader) {
                let slow = self.tricks.lock().slow.get(&token).copied();
                if let Some(a_kilobyte) = slow.filter(|_| kind == APPEND) {
                    thread::sleep(a_kilobyte * (bytes.len() >> 10) as u32);
                }
                let far = self.tricks.lock().far.get(&token).copied();
                if let Some(far) = far.filter(|_| kind == APPEND) {
                    thread::sleep(far);
                }
                let cut = self.tricks.lock().cut.get(&token).copied();
                if cut.is_some_and(|cut| kind == APPEND && bytes.len() > cut) {
                    break;
                }
                let mut projects = self.projects.lock();
                let stored = projects.get_mut(id);
                let seated = |stored: &&mut Stored| stored.seats.iter().any(|s| s.seat == seat);
                let Some(stored) = stored.filter(seated) else {
                    break;
                };
                let mut rest = &bytes[..];
                let mine = |seated: &&Seated| seated.seat == seat;
                let looking = stored.seats.iter().filter(mine).any(|s| s.mark & 2 != 0);
                match kind {
                    // Nothing is taken from someone who can only look.
                    APPEND if looking => {
                        for seated in stored.seats.iter().filter(mine) {
                            let _ = (&seated.socket).write_all(&frame(REFUSED, &[&[4]]));
                        }
                    }
                    APPEND => self.append(stored, seat, token, rest)?,
                    PRESENCE => stored.tell(seat, PRESENT, &[&seat.to_le_bytes(), rest]),
                    NOTE => {
                        let to = u32::from_le_bytes(number(&mut rest)?);
                        let said = frame(NOTED, &[&seat.to_le_bytes(), rest]);
                        for seated in stored.seats.iter().filter(|seated| seated.seat == to) {
                            let _ = (&seated.socket).write_all(&said);
                        }
                    }
                    _ => break,
                }
            }
            let mut projects = self.projects.lock();
            if let Some(stored) = projects.get_mut(id) {
                let before = stored.seats.len();
                stored.seats.retain(|seated| seated.seat != seat);
                stored.took(seat);
                if stored.seats.len() != before {
                    stored.people();
                }
            }
            Some(())
        }

        /// The heart of it: a batch goes in only if it names the head.
        fn append(
            &self,
            stored: &mut Stored,
            seat: u32,
            token: u64,
            mut bytes: &[u8],
        ) -> Option<()> {
            let base = u64::from_le_bytes(number(&mut bytes)?);
            let first_id = u64::from_le_bytes(number(&mut bytes)?);
            let count = u32::from_le_bytes(number(&mut bytes)?);
            let mut tricks = self.tricks.lock();
            if let Some(muted) = tricks.mute.get_mut(&token) {
                *muted += 1;
                return Some(());
            }
            let answer = |kind: u8, head: u64| {
                let seated = stored.seats.iter().find(|seated| seated.seat == seat)?;
                let said = frame(kind, &[&head.to_le_bytes()]);
                (&seated.socket).write_all(&said).ok()
            };
            if stored.frozen {
                let seated = stored.seats.iter().find(|seated| seated.seat == seat)?;
                return (&seated.socket).write_all(&frame(REFUSED, &[&[2]])).ok();
            }
            // Whoever was told stale has the next turn, for a few seconds at
            // most, and anyone else who sends meanwhile waits behind them.
            let now = Instant::now();
            while !stored.turns.is_empty() && stored.until.is_some_and(|until| now > until) {
                stored.turns.remove(0);
                stored.until = stored.until.map(|until| until + TURN);
            }
            let turn = stored.turns.first().copied();
            let (run, worst) = tricks.stale.entry(token).or_default();
            if base != stored.head || turn.is_some_and(|turn| turn != seat) {
                if !stored.turns.contains(&seat) {
                    if stored.turns.is_empty() {
                        stored.until = Some(now + TURN);
                    }
                    stored.turns.push(seat);
                }
                *run += 1;
                *worst = (*worst).max(*run);
                return answer(STALE, stored.head);
            }
            *run = 0;
            let kept: usize = stored.entries.iter().map(|entry| entry.2.len()).sum();
            if stored.room.is_some_and(|room| kept > room) {
                stored.turned += 1;
                let seated = stored.seats.iter().find(|seated| seated.seat == seat)?;
                return (&seated.socket).write_all(&frame(REFUSED, &[&[5]])).ok();
            }
            let first = base + 1;
            stored.entries.push((first, count, bytes.to_vec()));
            stored.head += u64::from(count);
            stored.heard.insert(token, first_id + u64::from(count) - 1);
            if let Some(at) = stored.turns.iter().position(|turn| *turn == seat) {
                stored.turns.remove(at);
                if at == 0 {
                    stored.until = Some(now + TURN);
                }
            }
            let header = [
                &first.to_le_bytes()[..],
                &count.to_le_bytes(),
                &seat.to_le_bytes(),
            ];
            stored.tell(seat, ENTRY, &[&header.concat(), bytes]);
            if tricks.deaf.contains(&token) {
                return Some(());
            }
            answer(ACK, stored.head)
        }

        /// Cut a machine off, as losing its network would, or let it back.
        fn shut(&self, token: u64, shut: bool) {
            if !shut {
                self.tricks.lock().shut.remove(&token);
                return;
            }
            self.tricks.lock().shut.insert(token);
            for stored in self.projects.lock().values() {
                for seated in stored.seats.iter().filter(|seated| seated.token == token) {
                    let _ = seated.socket.shutdown(Shutdown::Both);
                }
            }
        }

        /// Put a batch in the order as someone with the key and no manners
        /// would: whatever `sealed` makes for the head it is given, under
        /// whatever count is claimed for it. Everyone seated is sent it.
        fn forge(&self, id: &str, count: u32, sealed: impl FnOnce(u64) -> Vec<u8>) {
            let mut projects = self.projects.lock();
            let stored = projects.get_mut(id).unwrap();
            let (first, sealed) = (stored.head + 1, sealed(stored.head));
            stored.head += u64::from(count);
            let header = [&first.to_le_bytes()[..], &count.to_le_bytes(), &[0xff; 4]];
            stored.tell(0, ENTRY, &[&header.concat(), &sealed]);
            stored.entries.push((first, count, sealed));
        }

        /// Move a project to a new invite, as the server does when the
        /// host's interface asks it to with what [`Member::move_to`] gave.
        fn moved(&self, id: &str, body: &str) {
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            let text = |field: &str| body[field].as_str().unwrap().to_owned();
            let mut projects = self.projects.lock();
            let mut stored = projects.remove(id).unwrap();
            let removed = body["seat"].as_u64();
            stored.seats.retain(|seated| {
                let gone = Some(u64::from(seated.seat)) == removed;
                if gone {
                    let _ = (&seated.socket).write_all(&frame(REMOVED, &[]));
                    let _ = seated.socket.shutdown(Shutdown::Both);
                }
                !gone
            });
            stored.access = text("access");
            stored.made["wrap"] = body["wrap"].clone();
            stored.made["look"] = body["look"].clone();
            stored.made["look_wrap"] = body["look_wrap"].clone();
            for seated in &stored.seats {
                let theirs = match seated.mark & 2 {
                    0 => &body["moved"],
                    _ => &body["moved_look"],
                };
                let Some(theirs) = theirs.as_str() else {
                    continue;
                };
                let said = frame(MOVED, &[&hex::decode(theirs).unwrap()]);
                let _ = (&seated.socket).write_all(&said);
            }
            projects.insert(text("id"), stored);
        }

        /// Turn the invite that can only look on or off, as the server does
        /// when the host's interface asks it to with what
        /// [`Member::look_body`] gave. Off puts out whoever is here by it.
        fn looked(&self, id: &str, body: &str) {
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            let mut projects = self.projects.lock();
            let stored = projects.get_mut(id).unwrap();
            stored.made["look"] = body["look"].clone();
            stored.made["look_wrap"] = body["wrap"].clone();
            let on = !body["look"].is_null();
            stored.seats.retain(|seated| {
                let out = !on && seated.mark & 2 != 0;
                let said = match out {
                    true => frame(REMOVED, &[]),
                    false => frame(LOOKS, &[&[u8::from(on)]]),
                };
                let _ = (&seated.socket).write_all(&said);
                if out {
                    let _ = seated.socket.shutdown(Shutdown::Both);
                }
                !out
            });
            stored.people();
        }
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(20), "{what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn solid(store: &DocumentStore, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let item = ProjectItem::Solid(SolidDef {
            id,
            name: name.into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 1920,
            height: 1080,
            extra: serde_json::Map::new(),
        });
        let index = store.snapshot().items.len();
        let item = Box::new(item);
        store.commit(Op::AddItem { index, item }).unwrap();
        id
    }

    /// A composition to work in. Making one is an edit that names none.
    fn comp(store: &DocumentStore, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let made: Composition = serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "width": 1920,
            "height": 1080,
            "frame_rate": lumit_core::time::FrameRate::new(60, 1).unwrap(),
            "duration": lumit_core::time::Duration(lumit_core::time::Rational::new(30, 1).unwrap()),
            "background": LinearColour([0.0, 0.0, 0.0, 1.0]),
            "layers": [],
        }))
        .unwrap();
        let index = store.snapshot().items.len();
        let item = Box::new(ProjectItem::Composition(made));
        store.commit(Op::AddItem { index, item }).unwrap();
        id
    }

    /// An edit inside the composition `comp`.
    fn mix(store: &DocumentStore, comp: Uuid, on: bool) {
        store.commit(Op::SetSoundMix { comp, on }).unwrap();
    }

    /// Who changed what as a machine has it, without what is its alone:
    /// which of it is new there.
    fn changes(machine: &Machine) -> Vec<Sitting> {
        let list = machine.0.activity().into_iter();
        list.map(|sitting| Sitting {
            new: false,
            ..sitting
        })
        .collect()
    }

    fn renamed(id: Uuid, name: &str) -> Op {
        let name = name.into();
        Op::RenameItem { id, name }
    }

    fn rename(store: &DocumentStore, id: Uuid, name: &str) {
        store.commit(renamed(id, name)).unwrap();
    }

    fn name_of(store: &DocumentStore, id: Uuid) -> Option<String> {
        let document = store.snapshot();
        document.item(id).map(|item| item.name().to_owned())
    }

    fn quiet() -> Events {
        Arc::new(|_| {})
    }

    /// Somewhere for a machine's events to go, and what has come.
    fn hearing() -> (Events, Arc<Mutex<Vec<Event>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        (Arc::new(move |event| sink.lock().push(event)), heard)
    }

    /// One machine in a project: its member, and the store it edits.
    type Machine = (Member, Arc<DocumentStore>);

    /// Put a new project online, with `items` solids in it to edit.
    fn host(items: usize, events: Events) -> (Machine, Vec<Uuid>) {
        stand();
        let store = Arc::new(DocumentStore::new(Document::new()));
        let items = (0..items).map(|n| solid(&store, &format!("solid {n}")));
        let items = items.collect();
        let member = share(store.clone(), "A", None, None, events).unwrap();
        ((member, store), items)
    }

    fn joined(invite: &Invite, name: &str, events: Events) -> Machine {
        joined_as(seat(invite, name, VERSION, false), events)
    }

    /// Open a copy again as its file, `file`, holds it.
    fn reopened(file: &Document) -> (Document, Resuming) {
        resume(&mut file.clone(), Path::new("")).expect("the copy is still a member")
    }

    fn started(document: Document, resuming: Resuming, events: Events) -> Machine {
        let store = Arc::new(DocumentStore::new(document));
        (resuming.start(store.clone(), events).unwrap(), store)
    }

    /// What the server holds of a member's project, opened and read: its
    /// snapshot with every entry after it applied in order, and the edits
    /// those entries hold.
    fn held(member: &Member) -> (Document, Vec<Op>) {
        let (inner, id) = (&member.inner, member.access().0);
        let key = inner.seal.get().unwrap();
        let projects = stand().projects.lock();
        let stored = &projects[&id];
        let (seq, sealed) = stored.snapshot.as_ref().unwrap();
        let packed = open(key, &snapshot_data(*seq), sealed).unwrap();
        let json = miniz_oxide::inflate::decompress_to_vec(&packed).unwrap();
        let mut stored_at: Value = serde_json::from_slice(&json).unwrap();
        let mut document: Document = serde_json::from_value(stored_at["document"].take()).unwrap();
        let (mut head, mut all) = (*seq, Vec::new());
        for (first, count, sealed) in &stored.entries {
            let header = [&first.to_le_bytes()[..], &count.to_le_bytes(), &[0; 4]];
            let bytes = [&header.concat()[..], sealed].concat();
            let Entry { last, ops, .. } = entry(key, &inner.seat, &bytes, head).unwrap();
            for op in &ops {
                apply(&mut document, op).unwrap();
            }
            head = last;
            all.extend(ops);
        }
        assert_eq!(head, stored.head);
        (document, all)
    }

    /// Wait until every machine is in step with the server, with every
    /// edit it made answered, and then hold each one's document against the
    /// one the server holds. Answers the edits in the server's entries.
    fn settled(machines: &[&Machine]) -> Vec<Op> {
        let id = machines[0].0.access().0;
        let head = || stand().projects.lock().get(&id).map(|stored| stored.head);
        let settles = || {
            let head = head();
            machines.iter().all(|(member, store)| {
                let in_step = !member.away() && store.saving_apart().is_none();
                let here = member.inner.line.lock().head;
                in_step && store.unanswered() == 0 && Some(here) == head
            })
        };
        // Each machine as it stands, so a run that never settles says which
        // part of it is stuck.
        let state = || {
            let each: Vec<String> = machines
                .iter()
                .map(|(member, store)| {
                    format!(
                        "{} away {} apart {} unanswered {} at {}",
                        member.inner.seat.name,
                        member.away(),
                        store.saving_apart().is_some(),
                        store.unanswered(),
                        member.inner.line.lock().head,
                    )
                })
                .collect();
            format!("server at {:?}, {}", head(), each.join(", "))
        };
        let start = Instant::now();
        while !settles() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "every machine has everything and has had every edit answered: {}",
                state()
            );
            thread::sleep(Duration::from_millis(5));
        }
        let (theirs, edits) = held(&machines[0].0);
        for (member, store) in machines {
            let differs = footprint(&store.snapshot(), &theirs);
            let name = &member.inner.seat.name;
            assert!(
                differs.is_empty(),
                "{name} is not on the server's document: {differs:?}"
            );
        }
        edits
    }

    /// Two machines and then three edit the same things and different ones
    /// at once, each from a thread of its own. They all end on one document,
    /// which is the server's snapshot with every entry applied in order, and
    /// every edit is in it once. Names, presence and a note cross, sealed.
    /// Then the host takes one of them out and makes a new invite: the two
    /// still there carry on by it, and the old one finds nothing.
    #[test]
    fn machines_editing_at_once_end_on_one_document() {
        const EDITS: usize = 300;
        let (events, heard) = hearing();
        let (a, items) = host(1, events);
        let shared = items[0];
        let invite: Invite = a.0.invite().to_string().parse().unwrap();
        assert!(invite.is_online());
        let b = joined(&invite, "B", quiet());
        let busy = |machine: &Machine, who: &'static str| {
            let store = machine.1.clone();
            thread::spawn(move || {
                let mine = solid(&store, who);
                let mut made = vec![mine];
                for n in 1..EDITS {
                    match n % 3 {
                        0 => rename(&store, shared, &format!("{who} {n}")),
                        1 => rename(&store, mine, &format!("{who}'s own {n}")),
                        _ => made.push(solid(&store, &format!("{who} {n}"))),
                    }
                    // Let the others in every so often, so that the edits
                    // cross and do not just follow each other.
                    if n % 50 == 0 {
                        until("some of it is answered", || store.unanswered() < 25);
                    }
                }
                made
            })
        };
        let mut made = Vec::new();
        for work in [busy(&a, "a"), busy(&b, "b")] {
            made.extend(work.join().unwrap());
        }
        assert_eq!(settled(&[&a, &b]).len(), 2 * EDITS);

        let (events, removed) = hearing();
        let c = joined(&invite, "C", events);
        for work in [busy(&a, "a"), busy(&b, "b"), busy(&c, "c")] {
            made.extend(work.join().unwrap());
        }
        assert_eq!(settled(&[&a, &b, &c]).len(), 5 * EDITS, "each edit once");
        let document = a.1.snapshot();
        assert_eq!(document.items.len(), 1 + made.len());
        assert!(made.iter().all(|id| document.item(*id).is_some()));

        until("A sees who is here by name", || {
            let mut names: Vec<String> = a.0.people().into_iter().map(|p| p.name).collect();
            names.sort();
            names == ["A", "B", "C"]
        });
        until("everyone sees who the host is, and no other", || {
            let hosts = |machine: &&Machine| {
                let people = machine.0.people().into_iter();
                let hosts: Vec<String> = people.filter(|p| p.host).map(|p| p.name).collect();
                hosts == ["A"]
            };
            [&a, &b, &c].iter().all(hosts)
        });
        let comp = Some(Uuid::now_v7());
        c.0.set_presence(Presence {
            comp,
            ..Presence::default()
        });
        until("A sees where C is", || {
            a.0.people().iter().any(|p| p.presence.comp == comp)
        });
        // What C is changing is held for it for as long as it says so, which
        // it does again by itself, and is let go of once it says no more, as
        // a machine that has vanished does not.
        let held = vec!["opacity".to_owned()];
        c.0.set_presence(Presence {
            comp,
            changing: held.clone(),
            ..Presence::default()
        });
        let holds = |held: &[String]| {
            let people = a.0.people();
            let c = people.iter().find(|p| p.name == "C");
            c.is_some_and(|p| p.presence.changing == held)
        };
        let last = || a.0.inner.changing.lock().first().map(|(_, at)| *at);
        until("A sees what C is changing", || holds(&held));
        let first = last();
        until("C says so again", || last() > first);
        assert!(holds(&held));
        *c.0.inner.said.lock() = None;
        until("A lets go of it", || holds(&[]));
        let body = serde_json::json!({ "said": "to A alone" });
        let note = Event::Note {
            from: b.0.me(),
            body: body.clone(),
        };
        b.0.note(a.0.me(), body);
        until("A hears B's note", || heard.lock().contains(&note));

        let (old, id) = (a.0.invite(), a.0.access().0);
        let (new, body) = a.0.move_to(Some(c.0.me())).unwrap();
        stand().moved(&id, &body);
        a.0.moved(new.clone());
        until("C hears it was taken out", || {
            removed.lock().contains(&Event::Ended(Ending::Removed))
        });
        until("B was sent the new invite", || b.0.invite().key == new.key);
        rename(&a.1, shared, "after the move, by a");
        rename(&b.1, made[0], "after the move, by b");
        settled(&[&a, &b]);
        assert!(join(old, "C", None).is_err());
    }

    /// The server drops a machine, which edits on, and so do the others.
    /// When it is back, what it did that touches nothing of theirs is in
    /// everyone's document, and what touches something of theirs comes back
    /// as conflicts with theirs standing. Keeping mine puts its edit on
    /// every machine, and keeping theirs leaves things as they are. A
    /// machine whose edits the server will not take for now is away in the
    /// same way, and loses nothing by it.
    #[test]
    fn a_machine_that_was_away_merges_on_the_way_back() {
        let (a, items) = host(3, quiet());
        let (x, y, z) = (items[0], items[1], items[2]);
        let (events, heard) = hearing();
        let b = joined(&a.0.invite(), "B", events);
        let gone = comp(&a.1, "Titles");
        settled(&[&a, &b]);

        let token = b.0.inner.seat.token;
        stand().shut(token, true);
        until("B misses the server", || {
            heard.lock().contains(&Event::Away)
        });
        // An edit inside a composition that A deletes meanwhile has nowhere
        // to go. It is dropped, and B is told what it was to.
        mix(&b.1, gone, true);
        a.1.commit(Op::RemoveItem { id: gone }).unwrap();
        rename(&b.1, x, "b's x");
        rename(&b.1, y, "b's y");
        rename(&b.1, z, "b's z");
        let apart = solid(&b.1, "made away");
        rename(&a.1, x, "a's x");
        rename(&a.1, y, "a's y");
        let meanwhile = solid(&a.1, "made meanwhile");
        until("the server has what A did", || a.1.unanswered() == 0);

        stand().shut(token, false);
        // The same invite again, which only cuts the wait short.
        b.0.reinvite(b.0.invite());
        let back = Event::Back {
            held: 2,
            refused: 1,
            dropped: vec![crate::Dropped {
                step: "Sound mix".into(),
                item: Some("Titles".into()),
                layer: None,
            }],
        };
        let Event::Back { dropped, .. } = &back else {
            unreachable!()
        };
        until("B is back with two edits held and one dropped", || {
            heard.lock().contains(&back)
        });
        // Its own document from just before is kept, as a project of its
        // own with everything it had made in it and nothing of A's.
        let copy = b.1.snapshot().id;
        let before = crate::before_returns(copy);
        assert_eq!(before.len(), 1, "B has a way back");
        // And beside it, what that return dropped, by name.
        assert!(before[0].refused == 1 && before[0].dropped == *dropped);
        assert!(before[0].at > 0);
        let (mine, _) = lumit_project::open(&before[0].path).unwrap();
        assert_ne!(mine.id, copy, "nobody's copy of the shared project");
        let named = |id| mine.item(id).map(|item| item.name().to_owned());
        assert_eq!(named(x).as_deref(), Some("b's x"));
        assert_eq!(named(y).as_deref(), Some("b's y"));
        assert!(mine.item(apart).is_some() && mine.item(meanwhile).is_none());
        assert!(mine.comp(gone).is_some_and(|comp| comp.sound_mix));
        settled(&[&a, &b]);
        for store in [&a.1, &b.1] {
            assert_eq!(name_of(store, x).as_deref(), Some("a's x"));
            assert_eq!(name_of(store, y).as_deref(), Some("a's y"));
            assert_eq!(name_of(store, z).as_deref(), Some("b's z"));
            let document = store.snapshot();
            assert!(document.item(apart).is_some() && document.item(meanwhile).is_some());
        }

        assert_eq!(b.0.conflicts().len(), 2);
        assert_eq!(b.0.resolve(0, true), 0);
        assert_eq!(b.0.resolve(0, false), 0);
        assert!(b.0.conflicts().is_empty());
        settled(&[&a, &b]);
        for store in [&a.1, &b.1] {
            assert_eq!(name_of(store, x).as_deref(), Some("b's x"), "mine kept");
            assert_eq!(name_of(store, y).as_deref(), Some("a's y"), "theirs kept");
        }

        // The server stops taking edits, as it does when the host's Lumit
        // Pro ends. A machine that makes one is told, and gives the line up
        // so that the edit is kept on disk. It goes in when edits are taken
        // again.
        let id = a.0.access().0;
        let freeze = |frozen: bool| {
            stand().projects.lock().get_mut(&id).unwrap().frozen = frozen;
        };
        heard.lock().clear();
        freeze(true);
        rename(&b.1, z, "made while nothing was taken");
        until("B is told its edit was not taken", || {
            heard.lock().contains(&Event::Writable(false)) && b.0.away()
        });
        assert!(b.1.saving_apart().is_some(), "kept as edits made away are");
        freeze(false);
        b.0.reinvite(b.0.invite());
        until("B is told edits are taken again", || {
            heard.lock().contains(&Event::Writable(true))
        });
        settled(&[&a, &b]);
        assert_eq!(
            name_of(&a.1, z).as_deref(),
            Some("made while nothing was taken")
        );
        // That return dropped nothing, so the copy is the one from before.
        assert!(crate::before_returns(copy) == before);
        assert!(lumit_project::open(&before[0].path).unwrap().0 == mine);

        // B goes away again and comes back to another edit dropped, before
        // its person has dealt with the first. Both copies are kept: the
        // first as it was, since it alone holds what that return dropped,
        // and the one from just now. Then the person lets go of them.
        let again = comp(&a.1, "Credits");
        settled(&[&a, &b]);
        heard.lock().clear();
        stand().shut(token, true);
        until("B misses the server again", || {
            heard.lock().contains(&Event::Away)
        });
        mix(&b.1, again, true);
        a.1.commit(Op::RemoveItem { id: again }).unwrap();
        until("the server has that", || a.1.unanswered() == 0);
        stand().shut(token, false);
        b.0.reinvite(b.0.invite());
        until("B is back with that edit dropped", || {
            let back = |event: &Event| matches!(event, Event::Back { refused: 1, .. });
            heard.lock().iter().any(back)
        });
        let both = crate::before_returns(copy);
        assert_eq!(both.len(), 2);
        assert!(both[1] == before[0], "newest first");
        assert!(lumit_project::open(&both[1].path).unwrap().0 == mine);
        // Each names what its own return dropped, and not the other's.
        let named = |kept: &crate::BeforeReturn| {
            let items = kept.dropped.iter();
            items
                .map(|dropped| dropped.item.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(named(&both[0]), [Some("Credits".to_owned())]);
        assert_eq!(named(&both[1]), [Some("Titles".to_owned())]);
        let (latest, _) = lumit_project::open(&both[0].path).unwrap();
        assert!(latest.comp(again).is_some_and(|comp| comp.sound_mix));
        assert!(latest.comp(gone).is_none(), "which only the first has");
        crate::forget_before_return(copy);
        assert!(crate::before_returns(copy).is_empty());
    }

    /// A copy that closes is still a member when it is opened again, in a
    /// store of its own from what was kept on disk. Closed in step, it
    /// takes the server's document with nothing to merge. Closed with an
    /// edit the server took and never got to answer, that edit is not made
    /// a second time: made again it would land on top of what someone else
    /// has since done to the same thing, and come back as a conflict. The
    /// edit made after it goes in. Closed with an edit that never reached
    /// the server, that edit goes in, once.
    #[test]
    fn a_copy_closed_and_opened_again_carries_on() {
        let (a, items) = host(2, quiet());
        let (x, y) = (items[0], items[1]);
        let id = a.0.access().0;
        let b = joined(&a.0.invite(), "B", quiet());
        let before = solid(&b.1, "made before it closed");
        settled(&[&a, &b]);
        let (file, first) = (Document::clone(&b.1.snapshot()), b.0.inner.seat.token);
        drop(b);
        rename(&a.1, x, "renamed while it was closed");
        until("the server has it", || a.1.unanswered() == 0);

        let (events, heard) = hearing();
        let (document, resuming) = reopened(&file);
        assert!(resuming.left.is_none(), "it closed in step");
        assert_ne!(resuming.seat.token, first, "nothing was in doubt");
        let b = started(document, resuming, events);
        let merged = Event::Back {
            held: 0,
            refused: 0,
            dropped: Vec::new(),
        };
        until("it has the server's document", || {
            heard.lock().contains(&merged)
        });
        settled(&[&a, &b]);
        assert_eq!(
            name_of(&b.1, x).as_deref(),
            Some("renamed while it was closed")
        );
        assert!(b.1.snapshot().item(before).is_some());

        // The server takes an edit and its answer never comes.
        let token = b.0.inner.seat.token;
        let taken = renamed(y, "taken and never answered");
        stand().tricks.lock().deaf.insert(token);
        b.1.commit(taken.clone()).unwrap();
        let heard_of = || {
            let projects = stand().projects.lock();
            projects[&id].heard.get(&token).copied()
        };
        until("the server has taken it", || heard_of() == Some(1));
        let after = solid(&b.1, "made after it");
        let file = Document::clone(&b.1.snapshot());
        drop(b);
        stand().tricks.lock().deaf.remove(&token);
        rename(&a.1, y, "renamed after it");
        until("the server has that too", || a.1.unanswered() == 0);

        let (events, heard) = hearing();
        let (document, resuming) = reopened(&file);
        assert_eq!(resuming.seat.token, token, "the server is asked by it");
        assert!(document.item(after).is_some(), "what was kept is on it");
        let b = started(document, resuming, events);
        until("it is back with nothing held", || {
            heard.lock().contains(&merged)
        });
        let edits = settled(&[&a, &b]);
        let times = |op: &Op| edits.iter().filter(|edit| *edit == op).count();
        assert_eq!(times(&taken), 1, "not made a second time");
        assert!(a.1.snapshot().item(after).is_some());
        assert_eq!(name_of(&b.1, y).as_deref(), Some("renamed after it"));

        // An edit is sent and never reaches the server.
        let lost = renamed(y, "never arrived");
        stand().tricks.lock().mute.insert(token, 0);
        b.1.commit(lost.clone()).unwrap();
        until("it was sent", || {
            stand().tricks.lock().mute.get(&token) > Some(&0)
        });
        let file = Document::clone(&b.1.snapshot());
        drop(b);
        stand().tricks.lock().mute.remove(&token);

        let (events, heard) = hearing();
        let (document, resuming) = reopened(&file);
        assert_eq!(resuming.seat.token, token);
        let b = started(document, resuming, events);
        until("it is back again", || heard.lock().contains(&merged));
        let edits = settled(&[&a, &b]);
        assert_eq!(edits.iter().filter(|edit| **edit == lost).count(), 1);
        assert_eq!(name_of(&a.1, y).as_deref(), Some("never arrived"));
    }

    /// Once the head has run far enough past the snapshot a machine stores
    /// a new one, and the server drops what is under it. A machine that
    /// joins after that, from the new snapshot and the entries since, has
    /// the document of the one that was there for every edit. It also has
    /// who made those edits, which only the snapshot could have told it:
    /// the entries they were in are gone.
    #[test]
    fn a_machine_that_joins_after_a_new_snapshot_has_the_same_document() {
        let (a, items) = host(1, quiet());
        let id = a.0.access().0;
        for n in 0..=COMPACT_EDITS {
            rename(&a.1, items[0], &format!("name {n}"));
        }
        let stored = || {
            let projects = stand().projects.lock();
            let (seq, _) = projects[&id].snapshot.as_ref().unwrap();
            (
                *seq,
                projects[&id].entries.iter().map(|entry| entry.0).min(),
            )
        };
        until("a new snapshot is stored", || stored().0 > COMPACT_EDITS);
        let late = solid(&a.1, "made after the snapshot");
        settled(&[&a]);
        let (seq, oldest) = stored();
        assert!(oldest.is_some_and(|first| first > seq), "only what follows");

        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        assert!(b.1.snapshot().item(late).is_some());
        let told = b.0.activity();
        assert_eq!(changes(&b), changes(&a));
        assert_eq!(told.len(), 1, "one sitting, all of it A's");
        assert_eq!(u64::from(told[0].edits), COMPACT_EDITS + 2);
        assert!(told[0].new && told[0].who == a.0.who());
    }

    /// What is sealed opens only as what it was sealed as, whole, in its
    /// place, with its key. A piece with a bit flipped is refused, and so
    /// is a batch moved along the order, and so is a file that was cut
    /// short, had its pieces swapped, or is another project's. A file
    /// bigger than one piece comes back as it went in, and so does an empty
    /// one. Sealing can be stopped, and then leaves nothing behind.
    #[test]
    fn what_was_changed_cut_short_or_moved_does_not_open() {
        let key: Key = random().unwrap();
        let sealed = seal(&key, b"note", b"said").unwrap();
        assert_eq!(open(&key, b"note", &sealed).as_deref(), Some(&b"said"[..]));
        assert!(
            open(&key, b"name", &sealed).is_none(),
            "not as anything else"
        );
        let mut flipped = sealed.clone();
        flipped[NONCE] ^= 1;
        assert!(open(&key, b"note", &flipped).is_none());

        let seat = Seat::new(Invite::online(key, None), None, "A", 1, None);
        let ops = vec![Op::Batch { ops: Vec::new() }];
        let plain = serde_json::to_vec(&serde_json::json!({ "ops": ops })).unwrap();
        let packed = miniz_oxide::deflate::compress_to_vec(&plain, 6);
        let sealed = seal(&key, &batch_data(4, 1), &packed).unwrap();
        let at = |first: u64| {
            let header = [&first.to_le_bytes()[..], &1u32.to_le_bytes(), &[0; 4]];
            [&header.concat()[..], &sealed].concat()
        };
        assert_eq!(entry(&key, &seat, &at(5), 4).unwrap().ops, ops);
        let moved = entry(&key, &seat, &at(6), 5);
        assert!(matches!(moved, Err(ShareError::Unsafe)), "one place along");

        let folder = std::env::temp_dir().join(format!("lumit-online-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let (source, sealed, out) = (folder.join("in"), folder.join("sealed"), folder.join("out"));
        let (mine, other) = (Seal { key }, Seal { key: [7; 32] });
        let go = AtomicBool::new(false);
        let big: Vec<u8> = (0..(2 * PIECE + PIECE / 2) as u32)
            .map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        for bytes in [&[][..], &big[..]] {
            fs::write(&source, bytes).unwrap();
            let mut told = (0, 0);
            let mut hear = |done, total| told = (done, total);
            mine.seal_file(&source, &sealed, &go, &mut hear).unwrap();
            assert_eq!(told, (bytes.len() as u64, bytes.len() as u64));
            mine.open_file(&sealed, &out, &go, &mut |_, _| {}).unwrap();
            assert!(fs::read(&out).unwrap() == bytes);
            fs::remove_file(&out).unwrap();
        }

        let whole = fs::read(&sealed).unwrap();
        let (head, piece) = (SEALED.len() + 8, PIECE + TAG);
        let refused = |bytes: &[u8], seal: &Seal| {
            fs::write(&sealed, bytes).unwrap();
            let opened = seal.open_file(&sealed, &out, &go, &mut |_, _| {});
            opened.is_err() && !out.exists()
        };
        assert!(refused(&whole, &other), "another project's key");
        let mut flipped = whole.clone();
        flipped[head + piece + 5] ^= 1;
        assert!(refused(&flipped, &mine), "a bit flipped");
        assert!(
            refused(&whole[..head + 2 * piece], &mine),
            "its last piece gone"
        );
        assert!(
            refused(&whole[..whole.len() - 1], &mine),
            "cut short by a byte"
        );
        assert!(refused(&whole[..head], &mine), "nothing in it at all");
        let (first, second) = (
            &whole[head..head + piece],
            &whole[head + piece..head + 2 * piece],
        );
        let swapped = [&whole[..head], second, first, &whole[head + 2 * piece..]].concat();
        assert!(refused(&swapped, &mine), "two pieces swapped");

        fs::remove_file(&sealed).unwrap();
        let stop = AtomicBool::new(true);
        assert!(mine
            .seal_file(&source, &sealed, &stop, &mut |_, _| {})
            .is_err());
        assert!(!sealed.exists(), "stopped, and nothing left");
        let _ = fs::remove_dir_all(&folder);
    }

    /// Two machines edit, each watching the other, and neither is told any
    /// of it is new. A third joins later and is told all of it is, with who
    /// made how many edits in which compositions. Once its person has seen
    /// that, what they then watch happen is not new. Then one machine goes
    /// away while another edits. Back, it is told that what that one did
    /// meanwhile is new, and nothing else is: not what it had seen of the
    /// third, and not its own, though its edits made away only went in then.
    #[test]
    fn a_machine_is_told_who_changed_what_while_it_was_not_there() {
        let (events, heard) = hearing();
        let (a, items) = host(1, events);
        let (main, titles) = (comp(&a.1, "Main"), comp(&a.1, "Titles"));
        let b = joined(&a.0.invite(), "B", quiet());
        mix(&a.1, main, true);
        mix(&a.1, main, false);
        settled(&[&a, &b]);
        mix(&b.1, main, true);
        mix(&b.1, titles, true);
        rename(&b.1, items[0], "by b");
        settled(&[&a, &b]);
        assert!(heard.lock().contains(&Event::Activity));
        assert!(a.0.activity().iter().all(|sitting| !sitting.new));

        let c = joined(&a.0.invite(), "C", quiet());
        settled(&[&a, &b, &c]);
        let told = |sitting: &Sitting| {
            let (name, comps) = (sitting.name.clone(), sitting.comps.clone());
            (name, sitting.edits, comps, sitting.new)
        };
        let told: Vec<_> = c.0.activity().iter().map(told).collect();
        // A made two compositions, which names neither, and two edits in
        // one. B made two edits in the two and one that names none.
        let expected = [
            ("A".to_owned(), 4, vec![main], true),
            ("B".to_owned(), 3, vec![main, titles], true),
        ];
        assert_eq!(told, expected);
        assert_eq!(changes(&c), changes(&a));
        assert_eq!(changes(&b), changes(&a));

        c.0.seen();
        mix(&c.1, main, false);
        mix(&a.1, titles, false);
        settled(&[&a, &b, &c]);
        let told = c.0.activity();
        assert!(
            told.iter().all(|sitting| !sitting.new),
            "watched as it went"
        );
        assert_eq!((told[0].edits, &told[0].comps), (5, &vec![main, titles]));
        assert_eq!((told[2].who, told[2].edits), (c.0.who(), 1));

        a.0.seen();
        b.0.seen();
        let token = b.0.inner.seat.token;
        stand().shut(token, true);
        until("B misses the server", || b.0.away());
        mix(&b.1, titles, true);
        mix(&b.1, titles, false);
        mix(&a.1, main, true);
        until("the server has what A did", || a.1.unanswered() == 0);
        stand().shut(token, false);
        b.0.reinvite(b.0.invite());
        settled(&[&a, &b, &c]);
        let told = b.0.activity();
        let by = |who: Uuid| told.iter().find(|sitting| sitting.who == who).unwrap();
        assert!(by(a.0.who()).new, "what A did while B was away");
        assert!(!by(c.0.who()).new, "C did nothing meanwhile");
        assert!(
            !by(b.0.who()).new,
            "its own, though it went in on the way back"
        );
        assert_eq!(by(b.0.who()).edits, 5);
        assert!(a.0.activity().iter().all(|sitting| !sitting.new));
        assert_eq!(changes(&b), changes(&a));
    }

    /// Someone with the key sends a batch that says of itself what it
    /// likes: a name a megabyte long, a time in the year 9999, a count of
    /// five for one edit, nobody for who. The edit is made, since edits are
    /// what the order is for, and the list takes it for what it is: one
    /// edit, by a name cut to length, no later than now. Honest edits go on
    /// after it. Hundreds of sittings, each claimed to be someone new, do
    /// not make the list longer than it is kept, and one edit across more
    /// compositions than a sitting lists does not make it list more.
    #[test]
    fn a_batch_that_lies_about_itself_does_not_spoil_the_list() {
        let (a, items) = host(1, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        let (id, key) = (a.0.access().0, *a.0.inner.seal.get().unwrap());
        let forge = |batch: Value, count: u32| {
            let plain = serde_json::to_vec(&batch).unwrap();
            let packed = miniz_oxide::deflate::compress_to_vec(&plain, 6);
            let sealed = |base| seal(&key, &batch_data(base, count), &packed).unwrap();
            stand().forge(&id, count, sealed);
        };
        forge(
            serde_json::json!({
                "who": "nobody",
                "name": "x".repeat(1 << 20),
                "at": 253_402_300_799u64,
                "ops": [renamed(items[0], "forged")],
            }),
            5,
        );
        rename(&a.1, items[0], "honest, after it");
        settled(&[&a, &b]);
        for machine in [&a, &b] {
            let told = machine.0.activity();
            let forged = told.iter().find(|sitting| sitting.who.is_nil()).unwrap();
            assert_eq!(forged.name, "x".repeat(64));
            assert_eq!(forged.edits, 1);
            assert!(forged.began <= forged.ended && forged.ended <= activity::now());
        }

        // One person by their own account: twice more than ten minutes
        // after the time before, which is a new sitting each, and then ten
        // minutes to the second, which is not.
        let who = Uuid::now_v7();
        let one = |who: Uuid, at: u64| {
            let ops = [renamed(items[0], &format!("forged at {at}"))];
            serde_json::json!({ "who": who, "name": "n", "at": at, "ops": ops })
        };
        for at in [0, 601, 1202, 1802] {
            forge(one(who, at), 1);
        }
        settled(&[&a, &b]);
        let told = b.0.activity();
        let theirs = told.iter().filter(|sitting| sitting.who == who);
        let theirs: Vec<u32> = theirs.map(|sitting| sitting.edits).collect();
        assert_eq!(theirs, [1, 1, 2]);

        // And then a crowd, each of them someone new.
        for _ in 0..250 {
            forge(one(Uuid::now_v7(), 0), 1);
        }
        settled(&[&a, &b]);
        let told = a.0.activity();
        assert_eq!(told.len(), 200);
        assert_eq!(changes(&b), changes(&a));

        let comps: Vec<Uuid> = (0..40).map(|n| comp(&a.1, &format!("{n}"))).collect();
        let ops = comps.iter().map(|comp| Op::SetSoundMix {
            comp: *comp,
            on: true,
        });
        let ops = ops.collect();
        a.1.commit(Op::Batch { ops }).unwrap();
        settled(&[&a, &b]);
        let told = b.0.activity();
        let last = told.last().unwrap();
        assert_eq!((last.who, last.edits), (a.0.who(), 41));
        assert_eq!(last.comps, comps[..32]);
        assert_eq!(told.len(), 200);

        // And a machine that reads all of that back from the server comes
        // to the same list.
        let c = joined(&a.0.invite(), "C", quiet());
        settled(&[&a, &b, &c]);
        assert_eq!(changes(&c), changes(&a));
    }

    /// One machine has an edit to send at every chance, so every append of
    /// another's races one of its. The other still gets each of its edits
    /// in within a few rounds, because whoever just had an append go in
    /// rests a beat and whoever was told theirs was stale does not.
    #[test]
    fn a_machine_that_never_stops_editing_does_not_shut_another_out() {
        /// The most times in a row the other machine may be told its append
        /// was stale. Measured at 1 in every one of thirty runs. With no
        /// rest it was between 5 and 28, and grew for as long as the first
        /// machine went on.
        const ROUNDS: u32 = 4;
        let (a, items) = host(2, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        let stop = Arc::new(AtomicBool::new(false));
        let (store, stopping, x) = (a.1.clone(), stop.clone(), items[0]);
        let storm = thread::spawn(move || {
            for n in 0u64.. {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                // Always something waiting to be sent, without making edits
                // faster than anyone could apply them.
                if store.unanswered() < 16 {
                    rename(&store, x, &format!("storm {n}"));
                } else {
                    thread::yield_now();
                }
            }
        });
        for n in 0..20 {
            rename(&b.1, items[1], &format!("b {n}"));
            until("B's edit goes in", || b.1.unanswered() == 0);
        }
        stop.store(true, Ordering::Relaxed);
        storm.join().unwrap();
        settled(&[&a, &b]);
        let token = b.0.inner.seat.token;
        let worst = stand()
            .tricks
            .lock()
            .stale
            .get(&token)
            .map_or(0, |run| run.1);
        assert!(
            worst <= ROUNDS,
            "told {worst} times in a row that it was stale"
        );
    }

    /// A seat at the project `invite` names, for a machine that runs the
    /// Lumit `version`. `host` is whether the server knows it for a machine
    /// of the account that keeps the project.
    fn seat(invite: &Invite, name: &str, version: &'static str, host: bool) -> Seat {
        let seat = Seat {
            version,
            ..Seat::new(invite.clone(), None, name, token(), None)
        };
        if !host {
            stand().tricks.lock().guests.insert(seat.token);
        }
        seat
    }

    /// Put a new project online from a machine on an older Lumit, with one
    /// solid in it.
    fn old_host() -> (Machine, Uuid) {
        stand();
        let store = Arc::new(DocumentStore::new(Document::new()));
        let item = solid(&store, "solid");
        let invite = Invite::online(random().unwrap(), None);
        let seat = seat(&invite, "A", OLD, true);
        let member = share_as(store.clone(), seat, quiet()).unwrap();
        ((member, store), item)
    }

    fn joined_as(seat: Seat, events: Events) -> Machine {
        let (document, joining) = join_as(seat).unwrap();
        let store = Arc::new(DocumentStore::new(document));
        (joining.start(store.clone(), events).unwrap(), store)
    }

    /// The Lumit the server has the project `id` down as kept by.
    fn kept_by(id: &str) -> String {
        let projects = stand().projects.lock();
        projects[id].made["version"].as_str().unwrap().to_owned()
    }

    fn told(heard: &Mutex<Vec<Event>>, apart: &Versions) -> bool {
        heard.lock().contains(&Event::Versions(apart.clone()))
    }

    /// 0.10.0 comes after 0.9.0, which as text it does not. Got wrong, the
    /// day Lumit reaches 0.10.0 everyone on it is told to update to 0.9.0.
    #[test]
    fn versions_are_held_against_each_other_as_numbers() {
        let stands = |mine, theirs: &str, host| {
            let meta = Meta {
                protocol: PROTOCOL,
                version: theirs.into(),
                schema: SCHEMA_VERSION.into(),
                wrap: String::new(),
                host,
                writable: None,
                looks: false,
                head: None,
                snapshot: None,
            };
            stands(mine, &meta).unwrap()
        };
        let version = || "0.9.0".to_owned();
        let movable = Versions::Movable { version: version() };
        let first = Versions::HostFirst { version: version() };
        assert_eq!(stands("0.10.0", "0.9.0", true), Some(movable));
        assert_eq!(stands("0.10.0", "0.9.0", false), Some(first));
        let version = "0.10.0".to_owned();
        let update = Some(Versions::Update { version });
        assert_eq!(stands("0.9.0", "0.10.0", true), update);
        assert_eq!(stands("0.9.0", "0.9.0", true), None);
    }

    /// An older Lumit keeps a project, and its people update one at a time.
    /// A guest who updates first is told the host has to open it first, and
    /// stays a member, away. The host's machine is told it may move the
    /// project forward, and once its person agrees the project is this
    /// Lumit's: an edit made on the older one is in it, and the guest who
    /// was waiting joins by itself. A machine still at work on the older
    /// Lumit is cut off and told to update. The edit it had not had
    /// answered is kept, and goes in once that machine has updated too.
    #[test]
    fn a_newer_host_moves_an_older_project_forward() {
        let (a, item) = old_host();
        let (invite, id) = (a.0.invite(), a.0.access().0);
        let (events, heard_b) = hearing();
        let b = joined_as(seat(&invite, "B", OLD, false), events);
        let c = joined_as(seat(&invite, "C", OLD, false), quiet());
        rename(&c.1, item, "renamed on the older Lumit");
        settled(&[&a, &b, &c]);
        let file_a = Document::clone(&a.1.snapshot());
        let file_c = Document::clone(&c.1.snapshot());
        drop((a, c));

        // C's person updates first.
        let (events, heard_c) = hearing();
        let (document, resuming) = reopened(&file_c);
        stand().tricks.lock().guests.insert(resuming.seat.token);
        let c = started(document, resuming, events);
        let first = Versions::HostFirst {
            version: OLD.into(),
        };
        until("C is told the host has to open it first", || {
            told(&heard_c, &first)
        });
        assert!(c.0.away() && !c.0.host());
        assert_eq!(c.0.versions().as_ref(), Some(&first));
        let moved = c.0.forward()();
        assert!(matches!(moved, Err(ShareError::Versions(apart)) if apart == first));
        assert_eq!(kept_by(&id), OLD);

        // B works on, on the older Lumit, and has an edit the server has
        // not answered when the project moves.
        let token = b.0.inner.seat.token;
        stand().tricks.lock().mute.insert(token, 0);
        let late = solid(&b.1, "made on the older Lumit");
        until("it was sent", || {
            stand().tricks.lock().mute.get(&token) > Some(&0)
        });

        // The host's person updates, opens the copy, and agrees.
        let (events, heard_a) = hearing();
        let (document, resuming) = reopened(&file_a);
        let a = started(document, resuming, events);
        let movable = Versions::Movable {
            version: OLD.into(),
        };
        until("A is told it can move the project forward", || {
            told(&heard_a, &movable)
        });
        assert!(a.0.host());
        assert_eq!(a.0.versions(), Some(movable));
        a.0.forward()().unwrap();
        assert_eq!(kept_by(&id), VERSION);
        until("A is back", || !a.0.away());
        assert_eq!(a.0.versions(), None);
        // C asks every so often, and less often as time goes on. That is
        // not waited out here.
        c.0.inner.hurry.store(true, Ordering::Relaxed);
        until("C joined by itself", || !c.0.away());
        let after = settled(&[&a, &c]);
        assert!(after.is_empty(), "what was stored is the head");
        assert_eq!(
            name_of(&a.1, item).as_deref(),
            Some("renamed on the older Lumit")
        );

        let update = Versions::Update {
            version: VERSION.into(),
        };
        until("B is told to update", || told(&heard_b, &update));
        assert!(b.0.away());
        let file_b = Document::clone(&b.1.snapshot());
        drop(b);
        stand().tricks.lock().mute.remove(&token);
        let (document, resuming) = reopened(&file_b);
        assert!(document.item(late).is_some(), "what was kept is on it");
        let b = started(document, resuming, quiet());
        settled(&[&a, &b, &c]);
        assert!(a.1.snapshot().item(late).is_some());
    }

    /// Only the host's machine moves a project forward. A guest on a newer
    /// Lumit is told the host has to open it first, when it asks to join
    /// and when it asks to move the project, and the server would not take
    /// the snapshot from it either. The project is left the older Lumit's,
    /// with the machine on that still at work in it.
    #[test]
    fn a_newer_guest_alone_cannot_move_a_project_forward() {
        let (a, item) = old_host();
        let (invite, id) = (a.0.invite(), a.0.access().0);
        let guest = || seat(&invite, "B", VERSION, false);
        let first = Versions::HostFirst {
            version: OLD.into(),
        };
        let refused = |answer: Result<(), ShareError>| {
            assert!(matches!(answer, Err(ShareError::Versions(apart)) if apart == first));
        };
        refused(join_as(guest()).map(drop));
        let stop = AtomicBool::new(false);
        refused(carry_forward(&guest(), &stop));

        let key = a.0.inner.seal.get().unwrap();
        let sealed = sealed_snapshot(key, &guest(), &a.1.snapshot(), (&[], &[]), 0).unwrap();
        let door = &mut dial(&guest()).unwrap();
        let put = put_snapshot(door, 0, &sealed, Some(VERSION), &stop);
        assert!(matches!(put, Err(ShareError::Server(code)) if code == "forbidden"));

        assert_eq!(kept_by(&id), OLD);
        rename(&a.1, item, "still the older Lumit's");
        settled(&[&a]);
    }

    /// The edits an older Lumit stored are applied by this one to move its
    /// project forward. If one of them will not apply, the move stops and
    /// says so, and nothing at the server has changed: the project is the
    /// older Lumit's, as it was.
    #[test]
    fn a_project_this_lumit_cannot_read_is_not_moved() {
        let (a, _) = old_host();
        let (invite, id) = (a.0.invite(), a.0.access().0);
        let key = *a.0.inner.seal.get().unwrap();
        drop(a);
        let ops = [renamed(Uuid::now_v7(), "nothing by that id")];
        let batch = serde_json::json!({ "who": Uuid::nil(), "name": "A", "at": 0, "ops": ops });
        let plain = serde_json::to_vec(&batch).unwrap();
        let packed = miniz_oxide::deflate::compress_to_vec(&plain, 6);
        stand().forge(&id, 1, |base| {
            seal(&key, &batch_data(base, 1), &packed).unwrap()
        });
        let stored = || {
            let projects = stand().projects.lock();
            let stored = &projects[&id];
            let (snapshot, entries) = (stored.snapshot.clone(), stored.entries.clone());
            (stored.made.clone(), snapshot, entries, stored.head)
        };
        let before = stored();
        let host = seat(&invite, "A", VERSION, true);
        let moved = carry_forward(&host, &AtomicBool::new(false));
        assert!(matches!(moved, Err(ShareError::Unsafe)));
        assert_eq!(stored(), before);
    }

    /// A project stored at an older schema is brought up to this Lumit's on
    /// the way forward, by the migrations a file of that schema is opened
    /// through. Here a camera's position was the point it looked at, and
    /// comes out as the eye, its zoom behind that point.
    ///
    /// The camera sits in a top-level `comps`, which is where those
    /// migrations look for one and where their own tests put theirs.
    #[test]
    fn a_snapshot_at_an_older_schema_is_migrated_as_a_file_is() {
        use lumit_core::anim::Property;
        use lumit_core::model::TransformGroup;
        stand();
        let transform = TransformGroup {
            position_x: Property::fixed(960.0),
            position_y: Property::fixed(540.0),
            position_z: Property::fixed(0.0),
            ..TransformGroup::default()
        };
        let camera = serde_json::json!({
            "kind": { "Camera": { "zoom": Property::fixed(1000.0) } },
            "transform": transform,
        });
        let mut document = Document::new();
        let comps = serde_json::json!([{ "layers": [camera] }]);
        document.extra.insert("comps".into(), comps);
        let store = Arc::new(DocumentStore::new(document));
        let invite = Invite::online(random().unwrap(), None);
        let a = share_as(store, seat(&invite, "A", OLD, true), quiet()).unwrap();
        let id = a.access().0;
        drop(a);
        stand().projects.lock().get_mut(&id).unwrap().made["schema"] = "0.2.0".into();

        let host = || seat(&invite, "A", VERSION, true);
        carry_forward(&host(), &AtomicBool::new(false)).unwrap();
        let made = stand().projects.lock()[&id].made.clone();
        assert_eq!(made["version"], VERSION);
        assert_eq!(made["schema"], SCHEMA_VERSION);
        let (document, _) = join_as(host()).unwrap();
        let layer = &document.extra["comps"][0]["layers"][0];
        let eye: TransformGroup = serde_json::from_value(layer["transform"].clone()).unwrap();
        assert!((eye.position_z.value_at(0.0) + 1000.0).abs() < 1e-9);
        let aim = layer["kind"]["Camera"]["options"]["point_of_interest"].clone();
        let aim: [Property; 3] = serde_json::from_value(aim).unwrap();
        assert_eq!(aim[0].value_at(0.0), 960.0);
        assert_eq!(aim[2].value_at(0.0), 0.0);
    }

    /// The invite that can only look is off until the host turns it on,
    /// and off it opens nothing, whoever holds it. On, whoever joins by it
    /// sees the project and the people, and is seen, marked as looking.
    /// What they make is taken back and reaches nobody, and the server
    /// takes nothing from them. The invite gives nothing of the one that
    /// edits. When the host makes a new invite they carry on by the new one
    /// of their own kind, and the old one finds nothing. Turned off, they
    /// are put out. A password guards editing only, and holding both
    /// invites gives nothing to try one against.
    #[test]
    fn the_invite_that_can_only_look_changes_nothing() {
        let (events, told) = hearing();
        let (a, items) = host(1, events);
        let x = items[0];
        let look = a.0.look_invite();
        assert!(look.look && look.to_string().parse::<Invite>().unwrap() == look);
        // What it comes to finds the same project, opens the key by a wrap
        // of its own, and shows the server something else, with nothing of
        // what an editor shows.
        let (edits, looks) = (a.0.inner.seat.keys(), keys(&look.key, &look.link, true));
        assert!(edits.id == looks.id && edits.name == looks.name);
        assert!(edits.look_wrap == looks.wrap && edits.wrap != looks.wrap);
        assert!(edits.look == looks.access && edits.access != looks.access);
        assert!(looks.told.is_none());

        // It is off until the host turns it on, and then it opens nothing.
        assert!(!a.0.looks() && join(look.clone(), "C", None).is_err());
        let turn = |on: bool| {
            let body = a.0.look_body(on).unwrap();
            stand().looked(&a.0.access().0, &body);
            until("the host is told", || {
                let said = |event: &Event| match event {
                    Event::LookInvite(on) => Some(*on),
                    _ => None,
                };
                a.0.looks() == on && told.lock().iter().rev().find_map(said) == Some(on)
            });
        };
        turn(true);

        let b = joined(&a.0.invite(), "B", quiet());
        let (events, heard) = hearing();
        let c = joined_as(seat(&look, "C", VERSION, false), events);
        assert!(c.0.look() && !b.0.look() && b.0.looks() && c.0.looks());
        assert!(
            c.0.look_invite() == look,
            "what it hands on is what it holds"
        );
        assert!(matches!(c.0.look_body(false), Err(ShareError::Unreachable)));
        settled(&[&a, &b, &c]);
        until("everyone sees who can only look", || {
            let looking = |machine: &&Machine| {
                let people = machine.0.people().into_iter();
                let looking: Vec<String> = people.filter(|p| p.look).map(|p| p.name).collect();
                looking == ["C"]
            };
            [&a, &b, &c].iter().all(looking)
        });

        // What it makes is taken back, and is in nobody's document. Its
        // person is told, once for a run of them.
        rename(&c.1, x, "by someone who can only look");
        until("it is taken back", || c.1.unanswered() == 0);
        let made = solid(&c.1, "made by someone who can only look");
        until("and the next", || c.1.unanswered() == 0);
        let said = |event: &&Event| **event == Event::LookOnly;
        assert_eq!(heard.lock().iter().filter(said).count(), 1, "for the two");
        rename(&a.1, x, "by the host");
        let edits = settled(&[&a, &b, &c]);
        assert_eq!(edits, [renamed(x, "by the host")]);
        assert!(c.1.snapshot().item(made).is_none());

        // And the server takes nothing from it, were it to send.
        let stop = AtomicBool::new(false);
        let (key, head) = (*c.0.inner.seal.get().unwrap(), c.0.inner.line.lock().head);
        let sealed =
            sealed_snapshot(&key, &c.0.inner.seat, &c.1.snapshot(), (&[], &[]), head).unwrap();
        let door = &mut dial(&c.0.inner.seat).unwrap();
        let put = put_snapshot(door, head, &sealed, None, &stop);
        assert!(matches!(put, Err(ShareError::Server(code)) if code == "look_only"));

        // A new invite replaces both and leaves this one on. Each machine
        // carries on by the new one of its own kind, and the old one that
        // looks finds nothing.
        let (invite, body) = a.0.move_to(None).unwrap();
        stand().moved(&a.0.access().0, &body);
        a.0.moved(invite);
        until("each was sent the new invite of its kind", || {
            let (edit, look) = (a.0.inner.seat.channel(), a.0.look_invite());
            let (b, c) = (b.0.invite(), c.0.invite());
            !b.look && b.key == edit && c.look && c.key == look.key
        });
        rename(&a.1, x, "after the move");
        settled(&[&a, &b, &c]);
        assert!(a.0.looks() && c.0.looks());
        assert!(matches!(c.0.move_to(None), Err(ShareError::Unreachable)));
        assert!(join(look, "D", None).is_err());

        // Turned off, whoever is looking is put out, and the invite opens
        // nothing again. Those who can edit are told and carry on.
        turn(false);
        until("C is put out", || {
            heard.lock().contains(&Event::Ended(Ending::Removed))
        });
        until("B is told", || !b.0.looks());
        let look = a.0.look_invite();
        assert!(join(look.clone(), "D", None).is_err());
        rename(&a.1, x, "with nobody looking");
        settled(&[&a, &b]);
        // A new invite made now leaves it off, and gives the server nothing
        // of it.
        let (invite, body) = a.0.move_to(None).unwrap();
        assert!(!body.contains("look"));
        stand().moved(&a.0.access().0, &body);
        a.0.moved(invite);
        until("B was sent the new invite", || {
            b.0.invite().key == a.0.inner.seat.channel()
        });
        settled(&[&a, &b]);
        assert!(join(a.0.look_invite(), "D", None).is_err());
        // On again, it opens the project again.
        turn(true);
        assert!(join(a.0.look_invite(), "D", None).is_ok());

        // A password guards editing and nothing else. Nothing of it is in
        // the invite that can only look, which is worked out from the link
        // alone: whoever holds both invites holds no more than the link,
        // and has nothing to try a password against.
        let lock = Lock::new("correct horse").unwrap();
        let invite = Invite::online(random().unwrap(), Some(&lock));
        let store = Arc::new(DocumentStore::new(Document::new()));
        let seat = Seat::new(invite.clone(), Some(lock), "A", token(), None);
        let locked = share_as(store, seat, quiet()).unwrap();
        let look = locked.look_invite();
        assert!(look.look && !look.locked);
        assert!(look.key == keys(&invite.link, &invite.link, false).read);
        let wrong = invite.clone().unlocked("wrong");
        let (right, guess) = (
            locked.inner.seat.keys(),
            keys(&wrong.key, &wrong.link, false),
        );
        assert!(right.read == guess.read && right.id == guess.id && right.look == guess.look);
        assert!(right.look_wrap == guess.look_wrap && right.name == guess.name);
        assert!(right.access != guess.access && right.wrap != guess.wrap);
        assert!(right.told != guess.told);

        // With that invite off, the locked link without its password
        // reaches nothing, taken for either kind of invite.
        let guesses = || {
            [
                Invite::online(invite.link, None),
                Invite::looking(invite.link),
                wrong.clone(),
                look.clone(),
            ]
        };
        let alone = join(invite.clone(), "C", None);
        assert!(matches!(alone, Err(ShareError::Locked)));
        assert!(guesses()
            .into_iter()
            .all(|guess| join(guess, "C", None).is_err()));
        // With it on, the link reads by that invite, which is the choice
        // made, and still edits by nothing but the password.
        let body = locked.look_body(true).unwrap();
        stand().looked(&locked.access().0, &body);
        let (_, looker) = join(look.clone(), "C", None).unwrap();
        assert!(looker.seat.look());
        for guess in guesses() {
            let opens = join(guess.clone(), "C", None).is_ok();
            assert_eq!(opens, guess == look);
        }
    }

    /// A note someone leaves reaches everyone once, in the project's own
    /// order, and is nobody's edit. A machine that was away finds what was
    /// said meanwhile new on the way back, and a note written while away
    /// waits, is kept with a copy that closes, and goes in when the server
    /// is found. Notes are kept through a new snapshot. A machine that can
    /// only look reads them and leaves none.
    #[test]
    fn a_note_reaches_everyone_once() {
        let (a, _) = host(1, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        let words = |machine: &Machine| -> Vec<String> {
            let notes = machine.0.notes().into_iter();
            notes.map(|note| note.text).collect()
        };
        let comp = Some(Uuid::now_v7());
        assert!(a.0.write_note("  the logo is early  ", comp, None, None));
        assert!(!a.0.write_note("   ", None, None, None), "no words");
        until("B has it", || words(&b) == ["the logo is early"]);
        settled(&[&a, &b]);
        assert!(a.0.notes() == b.0.notes());
        let note = b.0.notes().remove(0);
        assert!(note.comp == comp && note.name == "A" && note.last > 0 && !note.new);
        assert!(a.0.activity().is_empty() && b.0.activity().is_empty());

        // B is away and writes one, and A writes one meanwhile. B's copy
        // closes before the server is back.
        let token = b.0.inner.seat.token;
        stand().shut(token, true);
        until("B misses the server", || b.0.away());
        assert!(b.0.write_note("written while away", None, None, None));
        assert_eq!(b.0.notes().last().map(|note| note.last), Some(0));
        assert!(a.0.write_note("written meanwhile", None, None, None));
        until("the server has A's", || {
            a.0.notes().iter().all(|note| note.last > 0)
        });
        let file = Document::clone(&b.1.snapshot());
        drop(b);
        stand().shut(token, false);
        let (document, resuming) = reopened(&file);
        assert_eq!(resuming.notes.len(), 1, "kept with the copy");
        let b = started(document, resuming, quiet());
        until("each has all three", || {
            words(&a).len() == 3 && words(&b) == words(&a)
        });
        settled(&[&a, &b]);
        let said = [
            "the logo is early",
            "written meanwhile",
            "written while away",
        ];
        assert_eq!(words(&a), said);
        assert!(a.0.notes().iter().all(|note| note.last > 0));
        let new: Vec<bool> = b.0.notes().iter().map(|note| note.new).collect();
        assert_eq!(new, [false, true, false], "what A said while B was away");
        b.0.seen();
        assert!(b.0.notes().iter().all(|note| !note.new));

        // A new snapshot carries them, so a machine that joins after it has
        // them, and all of it is new there.
        let (id, head) = (a.0.access().0, a.0.inner.line.lock().head);
        a.0.inner.compact(head);
        until("the snapshot is stored", || {
            let projects = stand().projects.lock();
            let stored = projects[&id].snapshot.as_ref();
            stored.is_some_and(|(seq, _)| *seq == head) && projects[&id].entries.is_empty()
        });
        let c = joined(&a.0.invite(), "C", quiet());
        assert_eq!(words(&c), said);
        assert!(c.0.notes().iter().all(|note| note.new));

        // Someone who can only look reads them and leaves none.
        stand().looked(&id, &a.0.look_body(true).unwrap());
        let d = joined_as(seat(&a.0.look_invite(), "D", VERSION, false), quiet());
        assert_eq!(words(&d), said);
        assert!(!d.0.write_note("from someone looking", None, None, None));
        assert_eq!(words(&d), said);
    }

    /// Lumit's server keeps only so much of the order past its snapshot,
    /// and turns an edit away once it has that much. The machine it turns
    /// away stores a new snapshot, with its own edit still waiting to go
    /// in, and sends the edit again. Nothing anyone made is lost, with two
    /// machines at it at once and a third in the project that can only
    /// look, and so can store nothing.
    #[test]
    fn an_edit_turned_away_for_room_goes_in_after_a_new_snapshot() {
        let (a, _) = host(1, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        let id = a.0.access().0;
        stand().looked(&id, &a.0.look_body(true).unwrap());
        let d = joined_as(seat(&a.0.look_invite(), "D", VERSION, false), quiet());
        settled(&[&a, &b, &d]);
        // Room for one batch past a snapshot, so every batch after the
        // first is turned away until a new one is stored.
        stand().projects.lock().get_mut(&id).unwrap().room = Some(0);
        let mut made = Vec::new();
        for n in 0..8 {
            made.push(solid(&a.1, &format!("A's {n}")));
            made.push(solid(&b.1, &format!("B's {n}")));
        }
        // Each machine's document is held against the server's here, which
        // is its newest snapshot with the entries after it: so a snapshot
        // stored from under a waiting edit was the order's own document.
        settled(&[&a, &b, &d]);
        let (turned, snapshot) = {
            let projects = stand().projects.lock();
            let stored = &projects[&id];
            (stored.turned, stored.snapshot.as_ref().map(|at| at.0))
        };
        assert!(turned > 0, "the server did turn some away");
        assert!(snapshot > Some(0), "and a new snapshot was stored for it");
        // Whoever comes now starts from a snapshot stored that way.
        let c = joined(&a.0.invite(), "C", quiet());
        for machine in [&a, &b, &d, &c] {
            let document = machine.1.snapshot();
            assert_eq!(document.items.len(), 1 + made.len(), "each edit once");
            assert!(made.iter().all(|id| document.item(*id).is_some()));
        }
    }

    /// A project that has grown too large to store a snapshot of meets the
    /// server's ceiling for good: its edits are turned away for room, and
    /// no snapshot can make any. A machine that meets it says why, stays
    /// away with what it made kept on disk, and does not come back for
    /// nothing. Once someone has stored a snapshot it comes back, its edit
    /// goes in, and it says the project fits again.
    #[test]
    fn a_project_too_large_to_store_keeps_its_edits_and_says_why() {
        let (events, heard) = hearing();
        let (a, _) = host(1, events);
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        let id = a.0.access().0;
        let tight = |tight: bool| {
            let mut projects = stand().projects.lock();
            let stored = projects.get_mut(&id).unwrap();
            (stored.room, stored.tight) = (Some(0), tight);
        };
        tight(true);
        // There is room for one batch. The next is turned away, and the
        // snapshot that would make room for it does not fit.
        solid(&a.1, "goes in");
        until("the server has that", || a.1.unanswered() == 0);
        let mine = solid(&a.1, "turned away");
        until("A says why, and that nothing is taken", || {
            let heard = heard.lock();
            heard.contains(&Event::TooLarge(true)) && heard.contains(&Event::Writable(false))
        });
        until("A is away", || a.0.away());
        assert!(a.0.too_large() && !a.0.writable());
        assert_eq!(a.1.unanswered(), 1, "the edit is kept, not taken back");
        assert!(a.1.snapshot().item(mine).is_some());
        // B meets the same with an edit of its own, and its copy closes.
        let theirs = solid(&b.1, "B's, turned away");
        until("B is told too", || b.0.too_large() && b.0.away());
        let file = Document::clone(&b.1.snapshot());
        drop(b);
        // Asked to look again, A finds nothing has moved and stays away.
        a.0.reinvite(a.0.invite());
        thread::sleep(Duration::from_millis(500));
        let back = |event: &Event| matches!(event, Event::Back { .. });
        assert!(a.0.away() && !heard.lock().iter().any(back));

        // A snapshot fits again. B's copy, opened by a Lumit that has not
        // met the wall, has its edit from disk, stores one and gets it in.
        tight(false);
        let (document, resuming) = reopened(&file);
        let b = started(document, resuming, quiet());
        until("B's edit is in", || !b.0.away() && b.1.unanswered() == 0);
        // The head has moved, so A comes back and says the project fits.
        a.0.reinvite(a.0.invite());
        until("A says it fits again", || {
            heard.lock().contains(&Event::TooLarge(false))
        });
        settled(&[&a, &b]);
        for machine in [&a, &b] {
            let document = machine.1.snapshot();
            assert!(document.item(mine).is_some() && document.item(theirs).is_some());
        }
        assert!(!a.0.too_large() && a.0.writable());
    }

    /// A batch that takes long to cross a slow line misses its place for as
    /// long as anyone else is at work: by the time it is there the head has
    /// moved on, and the turn the server then keeps for it is over before
    /// all of it has gone up again. A machine told so sends half as much the
    /// next time, so what it made goes in a piece at a time while the other
    /// never stops, and none of it waits for them to.
    #[test]
    fn a_big_batch_from_a_slow_line_goes_in_while_another_keeps_editing() {
        let (a, items) = host(2, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        // All B is about to make takes longer to arrive than a turn lasts.
        // Half of it does not. The names are noise, which deflating makes
        // no shorter.
        let slow = Duration::from_millis(8);
        stand()
            .tricks
            .lock()
            .slow
            .insert(b.0.inner.seat.token, slow);
        let stop = Arc::new(AtomicBool::new(false));
        let (store, stopping, x) = (a.1.clone(), stop.clone(), items[0]);
        let storm = thread::spawn(move || {
            for n in 0u64.. {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                if store.unanswered() < 16 {
                    rename(&store, x, &format!("storm {n}"));
                } else {
                    thread::yield_now();
                }
            }
        });
        let noise = || hex::encode(random::<256>().unwrap());
        let made: Vec<Uuid> = (0..700).map(|_| solid(&b.1, &noise())).collect();
        until("all of it goes in while A is still at work", || {
            b.1.unanswered() == 0
        });
        stop.store(true, Ordering::Relaxed);
        storm.join().unwrap();
        settled(&[&a, &b]);
        let document = a.1.snapshot();
        assert!(made.iter().all(|id| document.item(*id).is_some()));
    }

    /// A note its writer or the host takes down is gone from every list
    /// where that stands in the order. A machine that was away loses it on
    /// the way back, a copy of it that machine still had waiting is not put
    /// back, and no snapshot brings it back. Nobody else can take it down,
    /// and a machine that can only look can take down none.
    #[test]
    fn a_note_taken_down_stays_down() {
        let (a, _) = host(1, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        let c = joined(&a.0.invite(), "C", quiet());
        settled(&[&a, &b, &c]);
        let words = |machine: &Machine| -> Vec<String> {
            let notes = machine.0.notes().into_iter();
            notes.map(|note| note.text).collect()
        };
        let id = |machine: &Machine, text: &str| {
            let mut notes = machine.0.notes().into_iter();
            notes.find(|note| note.text == text).map(|note| note.id)
        };
        assert!(a.0.write_note("the host's", None, None, None));
        assert!(b.0.write_note("B's own", None, None, None));
        assert!(b.0.write_note("B's, for the host", None, None, None));
        until("everyone has the three", || {
            [&a, &b, &c].iter().all(|machine| words(machine).len() == 3)
        });
        settled(&[&a, &b, &c]);
        // Its writer, and the host. Nobody else, and nobody twice.
        let own = id(&b, "B's own").unwrap();
        assert!(!c.0.take_note_down(own), "it is not C's");
        assert!(!b.0.take_note_down(id(&b, "the host's").unwrap()));
        assert!(b.0.take_note_down(own));
        assert!(!b.0.take_note_down(own), "it is down already");
        assert!(a.0.take_note_down(id(&a, "B's, for the host").unwrap()));
        until("everyone has the one left", || {
            [&a, &b, &c]
                .iter()
                .all(|machine| words(machine) == ["the host's"])
        });
        settled(&[&a, &b, &c]);

        // B writes one that the server takes and never answers, and loses
        // its line: the note is in the order, and B still has it waiting to
        // be sent. The host takes it down meanwhile.
        let token = b.0.inner.seat.token;
        stand().tricks.lock().deaf.insert(token);
        assert!(b.0.write_note("in doubt", None, None, None));
        until("the others have it", || {
            [&a, &c].iter().all(|machine| words(machine).len() == 2)
        });
        stand().shut(token, true);
        until("B misses the server", || b.0.away());
        stand().tricks.lock().deaf.remove(&token);
        assert!(a.0.take_note_down(id(&a, "in doubt").unwrap()));
        until("C loses it", || words(&c) == ["the host's"]);
        assert_eq!(words(&b).len(), 2, "B has not heard yet");
        // B comes back, loses it with everyone else, and does not put its
        // own copy back.
        stand().shut(token, false);
        b.0.reinvite(b.0.invite());
        until("B is back without it", || {
            !b.0.away() && words(&b) == ["the host's"]
        });
        settled(&[&a, &b, &c]);
        for machine in [&a, &b, &c] {
            assert_eq!(words(machine), ["the host's"]);
        }
        let waits = b.0.inner.activity.lock().waiting.clone();
        assert!(waits.is_empty(), "still to send: {waits:?}");

        // No snapshot brings one back, for a machine that joins after it.
        let (project, head) = (a.0.access().0, a.0.inner.line.lock().head);
        a.0.inner.compact(head);
        until("the snapshot is stored", || {
            let projects = stand().projects.lock();
            let stored = projects[&project].snapshot.as_ref();
            stored.is_some_and(|(seq, _)| *seq == head) && projects[&project].entries.is_empty()
        });
        let d = joined(&a.0.invite(), "D", quiet());
        assert_eq!(words(&d), ["the host's"]);

        // Someone who can only look takes down nothing.
        stand().looked(&project, &a.0.look_body(true).unwrap());
        let e = joined_as(seat(&a.0.look_invite(), "E", VERSION, false), quiet());
        assert!(!e.0.take_note_down(id(&e, "the host's").unwrap()));
        assert_eq!(words(&e), ["the host's"]);
    }

    /// A machine that comes back carries on from what it has: it is sent the
    /// entries after its own head, and does not fetch the snapshot again.
    /// Only when the server's snapshot has passed it does it fetch
    /// everything, as a machine that is new does. Either way it ends on the
    /// order's document, with what it made while away in it.
    #[test]
    fn a_machine_that_comes_back_fetches_only_what_it_has_not_got() {
        let (a, items) = host(2, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        let token = b.0.inner.seat.token;
        let fetched = || {
            let tricks = stand().tricks.lock();
            tricks.fetched.get(&token).copied().unwrap_or(0)
        };
        assert_eq!(fetched(), 1, "once, to join");
        let away = |mine: &str, theirs: &str| {
            stand().shut(token, true);
            until("B misses the server", || b.0.away());
            rename(&b.1, items[0], mine);
            rename(&a.1, items[1], theirs);
            until("the server has A's", || a.1.unanswered() == 0);
        };
        let back = |mine: &str, theirs: &str| {
            stand().shut(token, false);
            b.0.reinvite(b.0.invite());
            until("B is back", || !b.0.away());
            settled(&[&a, &b]);
            for store in [&a.1, &b.1] {
                assert_eq!(name_of(store, items[0]).as_deref(), Some(mine));
                assert_eq!(name_of(store, items[1]).as_deref(), Some(theirs));
            }
        };

        away("B's, away", "A's, meanwhile");
        back("B's, away", "A's, meanwhile");
        assert_eq!(fetched(), 1, "only what came after what it had");

        // Again, and this time a new snapshot is stored past where B stands.
        away("B's, away again", "A's, under a new snapshot");
        let (id, head) = (a.0.access().0, a.0.inner.line.lock().head);
        a.0.inner.compact(head);
        until("the snapshot is stored", || {
            let projects = stand().projects.lock();
            let stored = projects[&id].snapshot.as_ref();
            stored.is_some_and(|(seq, _)| *seq == head)
        });
        back("B's, away again", "A's, under a new snapshot");
        assert_eq!(fetched(), 2, "the snapshot had passed it");
    }

    /// A machine far from the server gets its edits in while a nearer one
    /// never stops. Told its batch was stale, it is given the next turn, and
    /// the other waits for it. Each of its edits is in within a bound, and
    /// none waits for the other to pause.
    #[test]
    fn a_far_machine_gets_its_turn_while_a_near_one_never_stops() {
        let (a, items) = host(2, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        // Each of B's batches is a fifth of a second in coming: four times
        // what A leaves between two of its own.
        let far = Duration::from_millis(200);
        stand().tricks.lock().far.insert(b.0.inner.seat.token, far);
        let stop = Arc::new(AtomicBool::new(false));
        let (store, stopping, x) = (a.1.clone(), stop.clone(), items[0]);
        let storm = thread::spawn(move || {
            for n in 0u64.. {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                if store.unanswered() < 16 {
                    rename(&store, x, &format!("storm {n}"));
                } else {
                    thread::yield_now();
                }
            }
        });
        for n in 0..8 {
            let began = Instant::now();
            rename(&b.1, items[1], &format!("b {n}"));
            until("B's edit goes in", || b.1.unanswered() == 0);
            let took = began.elapsed();
            assert!(took < Duration::from_secs(2), "edit {n} waited {took:?}");
        }
        stop.store(true, Ordering::Relaxed);
        storm.join().unwrap();
        settled(&[&a, &b]);
    }

    /// A machine that was away when its way in was shut is told why when it
    /// comes back, as it would have been had it been there, and does not go
    /// on looking for what is not coming back: the project no longer kept
    /// online, this machine removed, the invite that can only look turned
    /// off. One whose invite was replaced under it is told that once and
    /// stays a member: given the new invite, it is back with what it made.
    #[test]
    fn a_machine_away_when_its_way_in_was_shut_is_told_why() {
        let away = |events: Events| {
            let (a, items) = host(1, quiet());
            let b = joined(&a.0.invite(), "B", events);
            settled(&[&a, &b]);
            stand().shut(b.0.inner.seat.token, true);
            until("B is away", || b.0.away());
            (a, b, items[0])
        };
        let endings = [
            ("closed", Ending::Closed),
            ("removed", Ending::Removed),
            ("look_off", Ending::Removed),
        ];
        for (why, ending) in endings {
            let (events, heard) = hearing();
            let (_a, b, _) = away(events);
            stand().tricks.lock().gone.insert(b.0.access().1, why);
            stand().shut(b.0.inner.seat.token, false);
            let ended = Event::Ended(ending);
            until(why, || heard.lock().contains(&ended));
        }

        let (events, heard) = hearing();
        let (a, b, x) = away(events);
        rename(&b.1, x, "made while away");
        let (old, id) = (b.0.access().1, a.0.access().0);
        let (new, body) = a.0.move_to(None).unwrap();
        stand().moved(&id, &body);
        a.0.moved(new.clone());
        stand().tricks.lock().gone.insert(old, "moved");
        stand().shut(b.0.inner.seat.token, false);
        let told = || {
            let heard = heard.lock();
            heard.iter().filter(|e| **e == Event::Replaced).count()
        };
        until("B is told its invite was replaced", || told() == 1);
        assert!(b.0.away());
        b.0.reinvite(new);
        settled(&[&a, &b]);
        assert_eq!(name_of(&a.1, x).as_deref(), Some("made while away"));
        assert_eq!(told(), 1);
    }

    /// A batch too big for the socket is stored whole by a call of its own,
    /// and a line that drops under it has all of it to send again. A machine
    /// that loses one that way sends half as much the next time, and comes
    /// back knowing it. So what it made goes in a piece at a time, down a
    /// line that never carries the whole.
    #[test]
    fn a_big_batch_lost_with_the_line_goes_in_smaller() {
        let (a, _) = host(1, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        let token = b.0.inner.seat.token;
        // Away, B makes more than one socket message carries. Its names are
        // noise, which deflating makes no shorter. Back, all of it is one
        // batch, and nothing over 400 KiB crosses B's line in one piece.
        stand().shut(token, true);
        until("B is away", || b.0.away());
        let noise = || hex::encode(random::<1100>().unwrap());
        let made: Vec<Uuid> = (0..550).map(|_| solid(&b.1, &noise())).collect();
        stand().tricks.lock().cut.insert(token, 400 << 10);
        stand().shut(token, false);
        until("all of it goes in", || !b.0.away() && b.1.unanswered() == 0);
        settled(&[&a, &b]);
        let document = a.1.snapshot();
        assert!(made.iter().all(|id| document.item(*id).is_some()));
    }

    /// A snapshot that a newer one replaces while it is being fetched starts
    /// clean: what came of the first is thrown away, and the machine joins
    /// by the newer. Kept, the two would be one snapshot that does not open.
    /// A part with nothing in it, which is the door saying it is still
    /// asking, changes nothing.
    #[test]
    fn a_snapshot_replaced_while_it_was_fetched_starts_clean() {
        let (a, items) = host(1, quiet());
        let seat = seat(&a.0.invite(), "B", VERSION, false);
        stand().tricks.lock().again.insert(seat.token);
        let b = joined_as(seat, quiet());
        settled(&[&a, &b]);
        assert!(b.1.snapshot().item(items[0]).is_some());
    }

    /// The line goes between the snapshot and going live. A machine that has
    /// just fetched the whole snapshot keeps it and asks to go live again:
    /// it does not fetch the project a second time.
    #[test]
    fn a_snapshot_fetched_is_kept_when_the_line_goes_before_it_is_live() {
        let (a, items) = host(1, quiet());
        let seat = seat(&a.0.invite(), "B", VERSION, false);
        let token = seat.token;
        stand().tricks.lock().lost.insert(token);
        let b = joined_as(seat, quiet());
        settled(&[&a, &b]);
        assert!(b.1.snapshot().item(items[0]).is_some());
        assert_eq!(stand().tricks.lock().fetched[&token], 1);
    }

    /// A machine is removed while it cannot hear, and its copy is closed
    /// before it can. Opened again the copy goes by a new token, and the
    /// server knows the machine it removed by the old one. It is still told
    /// that it was removed, and not only that its invite was replaced. So
    /// is a copy closed a second time before it found the server.
    #[test]
    fn a_copy_removed_while_closed_is_told_so_when_it_is_opened_again() {
        let (a, _) = host(1, quiet());
        let b = joined(&a.0.invite(), "B", quiet());
        settled(&[&a, &b]);
        let (file, was, access) = (
            Document::clone(&b.1.snapshot()),
            b.0.inner.seat.token,
            b.0.access().1,
        );
        drop(b);
        {
            let mut tricks = stand().tricks.lock();
            tricks.gone.insert(access, "moved");
            tricks.removed.insert(was);
        }
        // Opened again with no way to the server, and closed before it had
        // found one.
        let (document, resuming) = reopened(&file);
        let again = resuming.seat.token;
        assert_ne!(again, was, "it goes by a new token");
        stand().shut(again, true);
        let b = started(document, resuming, quiet());
        until("B is looking for the server", || b.0.away());
        drop(b);

        let (events, heard) = hearing();
        let (document, resuming) = reopened(&file);
        let _b = started(document, resuming, events);
        until("B is told it was removed", || {
            heard.lock().contains(&Event::Ended(Ending::Removed))
        });
        assert!(!heard.lock().contains(&Event::Replaced));
    }
}
