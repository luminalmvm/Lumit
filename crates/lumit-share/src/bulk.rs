//! Footage between the people in a shared project, for whoever does not
//! have a file the others are cutting with.
//!
//! Edits are small and have to arrive in order, so they keep the connection
//! they always had. Files go down a second one that each guest opens to the
//! host with the same invite, where a long transfer holds nothing up. The
//! host is the middle: a guest asks it, and what the host has not got it
//! asks of a guest who has, keeps, and passes on.
//!
//! This sends and receives files and no more. What a file is made from, and
//! where it is kept, is the [`Footage`] it is given, which is the engine's.
//!
//! Threads: each link has one that reads, one that sends, and one that makes
//! the files it is asked for. A guest has one more that keeps its link up.

use crate::wire::{Receiver, Sender};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver as Queue, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// What one machine asks another for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Wanted {
    /// The whole of a footage item, made small, to cut with.
    StandIn { item: Uuid },
    /// `count` frames of a footage item from `first`, as good as the
    /// original, for an export that uses only those.
    Part { item: Uuid, first: u64, count: u64 },
    /// The footage item's file itself.
    Original { item: Uuid },
    /// The file an export this machine asked another to do came to.
    Export { job: Uuid },
}

impl Wanted {
    /// The footage item this is of, or `None` for an export.
    #[must_use]
    pub fn item(&self) -> Option<Uuid> {
        match self {
            Wanted::StandIn { item } | Wanted::Part { item, .. } | Wanted::Original { item } => {
                Some(*item)
            }
            Wanted::Export { .. } => None,
        }
    }
}

/// A footage item one machine has the original of, and how big that file
/// is, which is what fetching it would cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub item: Uuid,
    pub bytes: u64,
}

/// How a transfer is getting on, for whoever is watching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum News {
    /// `done` of `total` bytes have crossed. `sending` is this machine
    /// giving, and otherwise it is taking.
    Moving {
        wanted: Wanted,
        done: u64,
        total: u64,
        sending: bool,
    },
    /// The file is here, whole, at `path`.
    Arrived { wanted: Wanted, path: PathBuf },
    /// Nobody in the project could give it.
    Refused { wanted: Wanted },
    /// Who has the original of which footage changed.
    Holders,
}

/// The engine's side of sending footage. Called from the share threads with
/// no lock held, and never from the UI thread.
pub trait Footage: Send + Sync {
    /// The file for `wanted` on this machine, made now if it has to be.
    /// `None` when this machine has nothing to make it from. May take as
    /// long as an encode takes, and gives up once `stop` is set.
    fn make(&self, wanted: &Wanted, stop: &AtomicBool) -> Option<PathBuf>;

    /// Where the file for `wanted` is to be written as it arrives, or
    /// `None` when this machine will not take it.
    fn room(&self, wanted: &Wanted) -> Option<PathBuf>;

    /// Something happened to a transfer.
    fn told(&self, news: News);
}

/// How fast this machine sends and takes footage, in bytes a second. Nought
/// is as fast as the line goes. Shared by every transfer on the machine,
/// and changed while they run.
#[derive(Debug, Default)]
pub struct Limits {
    up: Bucket,
    down: Bucket,
}

impl Limits {
    pub fn set(&self, up: u64, down: u64) {
        self.up.limit.store(up, Ordering::Relaxed);
        self.down.limit.store(down, Ordering::Relaxed);
    }

    /// The limits as they stand: up, then down.
    #[must_use]
    pub fn get(&self) -> (u64, u64) {
        let read = |bucket: &Bucket| bucket.limit.load(Ordering::Relaxed);
        (read(&self.up), read(&self.down))
    }
}

/// One direction's allowance. It fills at the limit, holds a second's worth
/// at most, and whoever takes more than is in it waits the difference out.
#[derive(Debug)]
struct Bucket {
    limit: AtomicU64,
    /// When it was last filled, and how many bytes are in it. Below nought
    /// when the last taker was let go into debt.
    level: Mutex<(Instant, f64)>,
}

impl Default for Bucket {
    fn default() -> Self {
        Bucket {
            limit: AtomicU64::new(0),
            level: Mutex::new((Instant::now(), 0.0)),
        }
    }
}

impl Bucket {
    /// Take `bytes` out, waiting first for as long as keeps to the limit.
    /// Cut short once `stop` is set.
    fn take(&self, bytes: usize, stop: &AtomicBool) {
        let limit = self.limit.load(Ordering::Relaxed);
        if limit == 0 {
            return;
        }
        let limit = limit as f64;
        let wait = {
            let mut level = self.level.lock();
            let now = Instant::now();
            let filled = level.1 + now.duration_since(level.0).as_secs_f64() * limit;
            *level = (now, filled.min(limit) - bytes as f64);
            (-level.1 / limit).max(0.0)
        };
        let until = Instant::now() + Duration::from_secs_f64(wait.min(30.0));
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(20).min(until - Instant::now()));
        }
    }
}

/// The most of a file one frame carries. Under what one message of the
/// channel is cut into, so a limit is kept to closely.
const PIECE: usize = 48 << 10;

/// The longest frame read: a piece and its few bytes of heading.
const FRAME_LIMIT: u32 = 1 << 20;

/// How many things one link may be asked for and not yet have started on.
const ASKED_ROOM: usize = 64;

/// How many frames wait to be sent on one link.
const OUT_ROOM: usize = 256;

/// How often progress is told, at most.
const TELLING: Duration = Duration::from_millis(250);

/// How often a quiet link says it is still there, and how long one that
/// says nothing is believed.
const PING: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_secs(30);

const WANT: u8 = 1;
const SIZE: u8 = 2;
const DATA: u8 = 3;
const END: u8 = 4;
const NO: u8 = 5;
const STILL_HERE: u8 = 6;

/// A frame's heading: what it is and which request it belongs to.
fn frame(kind: u8, request: u64, rest: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(9 + rest.len());
    bytes.push(kind);
    bytes.extend_from_slice(&request.to_le_bytes());
    bytes.extend_from_slice(rest);
    bytes
}

/// Where a file that was part-way here is picked up: how many bytes of it
/// this machine has, and what they hash to. The other end carries on from
/// there only if its file starts with the same bytes.
type Resume = (u64, [u8; 32]);

/// The hash of the first `length` bytes of the file at `path`.
fn start_of(path: &Path, length: u64) -> Option<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    let mut file = File::open(path).ok()?.take(length);
    let mut piece = vec![0u8; PIECE];
    let mut left = length;
    while left > 0 {
        let n = file.read(&mut piece).ok().filter(|n| *n > 0)?;
        hasher.update(&piece[..n]);
        left -= n as u64;
    }
    Some(*hasher.finalize().as_bytes())
}

/// What waits to go out on a link.
enum Out {
    Frame(Vec<u8>),
    /// The file at `path`, as the answer to `request`.
    File {
        request: u64,
        wanted: Wanted,
        path: PathBuf,
        resume: Resume,
    },
}

/// The way to one other machine.
struct Link {
    /// Which link this is, to tell it from the one that replaces it.
    id: u64,
    /// The guest's number on the host. On a guest, 0 for the host.
    peer: u32,
    out: SyncSender<Out>,
    make: SyncSender<(u64, Resume, Wanted)>,
    socket: TcpStream,
}

/// Something this machine asked for and is taking.
struct Taking {
    wanted: Wanted,
    peer: u32,
    path: PathBuf,
    file: Option<File>,
    done: u64,
    total: u64,
    told: Instant,
}

/// Something a guest asked the host for that the host is still getting.
struct Owed {
    wanted: Wanted,
    peer: u32,
    request: u64,
    resume: Resume,
}

/// One end's footage: the links it has, what it is taking, and on the host
/// what it owes.
pub(crate) struct Hub {
    footage: Arc<dyn Footage>,
    limits: Arc<Limits>,
    host: bool,
    stop: AtomicBool,
    next: AtomicU64,
    /// One a guest on the host, one at most on a guest.
    links: Mutex<Vec<Link>>,
    /// By request number. An entry goes when its file is whole or refused.
    taking: Mutex<HashMap<u64, Taking>>,
    /// Bounded by [`ASKED_ROOM`] a link, and emptied as files arrive.
    owed: Mutex<Vec<Owed>>,
    /// Who has the original of which footage item, by person. An entry goes
    /// when the person does.
    holds: Mutex<HashMap<u32, Vec<Held>>>,
    /// What a guest wants and has no link to ask down yet.
    waiting: Mutex<Vec<Wanted>>,
}

/// The file a transfer is written to until it is whole.
fn part(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

impl Hub {
    pub(crate) fn new(footage: Arc<dyn Footage>, limits: Arc<Limits>, host: bool) -> Arc<Hub> {
        Arc::new(Hub {
            footage,
            limits,
            host,
            stop: AtomicBool::new(false),
            next: AtomicU64::new(1),
            links: Mutex::new(Vec::new()),
            taking: Mutex::new(HashMap::new()),
            owed: Mutex::new(Vec::new()),
            holds: Mutex::new(HashMap::new()),
            waiting: Mutex::new(Vec::new()),
        })
    }

    /// Stop every transfer and let go of every link.
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        for link in self.links.lock().drain(..) {
            let _ = link.socket.shutdown(Shutdown::Both);
        }
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Note which originals `peer` has, as it said or as the host passed on.
    pub(crate) fn holds(&self, peer: u32, items: Vec<Held>) {
        self.holds.lock().insert(peer, items);
        self.footage.told(News::Holders);
    }

    /// Everyone's originals as this end knows them, to pass on.
    pub(crate) fn all_holds(&self) -> Vec<(u32, Vec<Held>)> {
        let holds = self.holds.lock();
        holds
            .iter()
            .map(|(peer, items)| (*peer, items.clone()))
            .collect()
    }

    /// Who has the original of `item`, and how big it is on their disk.
    pub(crate) fn holders(&self, item: Uuid) -> Vec<(u32, u64)> {
        let holds = self.holds.lock();
        let mut who: Vec<(u32, u64)> = holds
            .iter()
            .filter_map(|(peer, items)| {
                let held = items.iter().find(|held| held.item == item)?;
                Some((*peer, held.bytes))
            })
            .collect();
        who.sort_unstable();
        who
    }

    /// `peer` has gone, and what it was known to have with it.
    pub(crate) fn forget(&self, peer: u32) {
        self.holds.lock().remove(&peer);
        self.footage.told(News::Holders);
    }

    fn out(&self, peer: u32) -> Option<SyncSender<Out>> {
        let links = self.links.lock();
        links.iter().find(|l| l.peer == peer).map(|l| l.out.clone())
    }

    /// Ask for `wanted`: of the host on a guest, and on the host of whichever
    /// guest has the original. Asked once however often this is called.
    pub(crate) fn want(self: &Arc<Self>, wanted: Wanted) {
        if self.taking.lock().values().any(|t| t.wanted == wanted) {
            return;
        }
        if !self.host {
            if self.out(0).is_none() {
                let mut waiting = self.waiting.lock();
                if !waiting.contains(&wanted) {
                    waiting.push(wanted);
                }
                return;
            }
            self.ask(0, wanted);
            return;
        }
        if !self.fetch(&wanted, None) {
            self.footage.told(News::Refused { wanted });
        }
    }

    /// On the host: ask a guest that has the original for `wanted`. Not
    /// `but`, who is the one asking. False when nobody has it.
    fn fetch(self: &Arc<Self>, wanted: &Wanted, but: Option<u32>) -> bool {
        if self.taking.lock().values().any(|t| t.wanted == *wanted) {
            return true;
        }
        let Some(item) = wanted.item() else {
            return false;
        };
        let linked: Vec<u32> = self.links.lock().iter().map(|l| l.peer).collect();
        let holders = self.holders(item).into_iter().map(|(peer, _)| peer);
        let holder = holders
            .into_iter()
            .find(|peer| Some(*peer) != but && linked.contains(peer));
        match holder {
            Some(peer) => self.ask(peer, wanted.clone()),
            None => false,
        }
    }

    /// Send `peer` a request for `wanted`, picking a file up where it was
    /// left if part of it is here already. False when it could not be asked.
    fn ask(self: &Arc<Self>, peer: u32, wanted: Wanted) -> bool {
        let Some(path) = self.footage.room(&wanted) else {
            return false;
        };
        let (Some(out), Ok(text)) = (self.out(peer), serde_json::to_vec(&wanted)) else {
            return false;
        };
        let have = fs::metadata(part(&path)).map_or(0, |m| m.len());
        let (have, start) = match start_of(&part(&path), have) {
            Some(start) if have > 0 => (have, start),
            _ => (0, [0u8; 32]),
        };
        let request = self.next.fetch_add(1, Ordering::Relaxed);
        let taking = Taking {
            wanted,
            peer,
            path,
            file: None,
            done: have,
            total: 0,
            told: Instant::now(),
        };
        self.taking.lock().insert(request, taking);
        let mut rest = have.to_le_bytes().to_vec();
        rest.extend_from_slice(&start);
        rest.extend_from_slice(&text);
        if out
            .try_send(Out::Frame(frame(WANT, request, &rest)))
            .is_err()
        {
            self.taking.lock().remove(&request);
            return false;
        }
        true
    }

    /// Run a link to `peer` until it drops. `sender` and `receiver` are a
    /// connection that has already proved both ends hold the invite.
    pub(crate) fn link(
        self: &Arc<Self>,
        peer: u32,
        socket: TcpStream,
        sender: Sender,
        mut receiver: Receiver,
    ) {
        let (out, outbox) = sync_channel(OUT_ROOM);
        let (make, asked) = sync_channel(ASKED_ROOM);
        let Ok(kept) = socket.try_clone() else {
            return;
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut links = self.links.lock();
            // The link this machine had before is over, noticed yet or not.
            links.retain(|l| {
                if l.peer == peer {
                    let _ = l.socket.shutdown(Shutdown::Both);
                }
                l.peer != peer
            });
            links.push(Link {
                id,
                peer,
                out: out.clone(),
                make,
                socket: kept,
            });
        }
        let sending = self.clone();
        let sends = thread::Builder::new()
            .name("lumit-share-bulk-send".into())
            .spawn(move || sending.send_loop(sender, &outbox));
        let making = self.clone();
        let reply = out.clone();
        let makes = thread::Builder::new()
            .name("lumit-share-bulk-make".into())
            .spawn(move || making.make_loop(peer, &asked, &reply));
        if sends.is_ok() && makes.is_ok() {
            // What was being taken when the last link dropped is asked for
            // again from where it stopped, and what waited for a link too.
            let again: Vec<Wanted> = {
                let mut taking = self.taking.lock();
                let mine: Vec<u64> = taking
                    .iter()
                    .filter(|(_, t)| t.peer == peer)
                    .map(|(request, _)| *request)
                    .collect();
                mine.iter()
                    .filter_map(|request| taking.remove(request))
                    .map(|t| t.wanted)
                    .collect()
            };
            let waited = std::mem::take(&mut *self.waiting.lock());
            for wanted in again.into_iter().chain(waited) {
                self.ask(peer, wanted);
            }
            receiver.patience(QUIET);
            self.read_loop(peer, &mut receiver);
        }
        let _ = socket.shutdown(Shutdown::Both);
        self.links.lock().retain(|l| l.id != id);
        if self.host {
            self.dropped(peer);
        }
    }

    /// On the host: `peer`'s link has gone. What it was giving is asked of
    /// someone else, and what it was owed is let go of.
    fn dropped(self: &Arc<Self>, peer: u32) {
        self.owed.lock().retain(|o| o.peer != peer);
        let lost: Vec<Wanted> = {
            let mut taking = self.taking.lock();
            let mine: Vec<u64> = taking
                .iter()
                .filter(|(_, t)| t.peer == peer)
                .map(|(request, _)| *request)
                .collect();
            mine.iter()
                .filter_map(|request| taking.remove(request))
                .map(|t| t.wanted)
                .collect()
        };
        for wanted in lost {
            if !self.stopped() && !self.fetch(&wanted, Some(peer)) {
                self.refuse(&wanted);
            }
        }
    }

    /// Nobody can give `wanted`: say so here, and to whoever was owed it.
    fn refuse(&self, wanted: &Wanted) {
        let owed: Vec<Owed> = {
            let mut all = self.owed.lock();
            let (gone, kept) = all.drain(..).partition(|o| o.wanted == *wanted);
            *all = kept;
            gone
        };
        for owed in owed {
            if let Some(out) = self.out(owed.peer) {
                let _ = out.try_send(Out::Frame(frame(NO, owed.request, &[])));
            }
        }
        self.footage.told(News::Refused {
            wanted: wanted.clone(),
        });
    }

    /// Make what a link asks for, one thing at a time, and queue it to go.
    fn make_loop(
        self: &Arc<Self>,
        peer: u32,
        asked: &Queue<(u64, Resume, Wanted)>,
        out: &SyncSender<Out>,
    ) {
        while let Ok((request, resume, wanted)) = asked.recv() {
            if self.stopped() {
                break;
            }
            if let Some(path) = self.footage.make(&wanted, &self.stop) {
                let file = Out::File {
                    request,
                    wanted,
                    path,
                    resume,
                };
                if out.send(file).is_err() {
                    break;
                }
                continue;
            }
            // A guest has only what it has. The host can ask someone else.
            let fetching = self.host && {
                self.owed.lock().push(Owed {
                    wanted: wanted.clone(),
                    peer,
                    request,
                    resume,
                });
                self.fetch(&wanted, Some(peer))
            };
            if !fetching {
                self.owed
                    .lock()
                    .retain(|o| o.request != request || o.peer != peer);
                if out.send(Out::Frame(frame(NO, request, &[]))).is_err() {
                    break;
                }
            }
        }
    }

    /// Send what is queued for a link: frames as they come, and a file a
    /// piece at a time with any frame that turns up let past between pieces.
    fn send_loop(&self, mut sender: Sender, outbox: &Queue<Out>) {
        let mut files: VecDeque<(u64, Wanted, PathBuf, Resume)> = VecDeque::new();
        let mut said = Instant::now();
        'link: loop {
            let next = if files.is_empty() {
                match outbox.recv_timeout(PING) {
                    Ok(next) => Some(next),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                outbox.try_recv().ok()
            };
            match next {
                Some(Out::Frame(bytes)) => {
                    if sender.send(&bytes).is_err() {
                        break;
                    }
                    said = Instant::now();
                    continue;
                }
                Some(Out::File {
                    request,
                    wanted,
                    path,
                    resume,
                }) => {
                    files.push_back((request, wanted, path, resume));
                    continue;
                }
                None if files.is_empty() => {
                    if said.elapsed() >= PING && sender.send(&[STILL_HERE]).is_err() {
                        break;
                    }
                    said = Instant::now();
                    continue;
                }
                None => {}
            }
            // One whole file, unless a frame wants out between its pieces.
            let Some((request, wanted, path, (have, start))) = files.pop_front() else {
                continue;
            };
            let opened = File::open(&path).and_then(|mut file| {
                let total = file.metadata()?.len();
                // Carried on from where the other end got to only if what
                // it has is how this file starts.
                let same = have > 0 && have <= total && start_of(&path, have) == Some(start);
                let from = if same { have } else { 0 };
                file.seek(SeekFrom::Start(from))?;
                Ok((file, total, from))
            });
            let Ok((mut file, total, from)) = opened else {
                if sender.send(&frame(NO, request, &[])).is_err() {
                    break;
                }
                continue;
            };
            let mut sizes = total.to_le_bytes().to_vec();
            sizes.extend_from_slice(&from.to_le_bytes());
            if sender.send(&frame(SIZE, request, &sizes)).is_err() {
                break;
            }
            let mut done = from;
            let mut told = Instant::now();
            let mut piece = vec![0u8; PIECE];
            loop {
                if self.stopped() {
                    break 'link;
                }
                while let Ok(between) = outbox.try_recv() {
                    match between {
                        Out::Frame(bytes) => {
                            if sender.send(&bytes).is_err() {
                                break 'link;
                            }
                        }
                        Out::File {
                            request,
                            wanted,
                            path,
                            resume,
                        } => files.push_back((request, wanted, path, resume)),
                    }
                }
                let Ok(n) = file.read(&mut piece) else {
                    break 'link;
                };
                if n == 0 {
                    break;
                }
                self.limits.up.take(n, &self.stop);
                if sender.send(&frame(DATA, request, &piece[..n])).is_err() {
                    break 'link;
                }
                done += n as u64;
                if told.elapsed() >= TELLING {
                    told = Instant::now();
                    self.footage.told(News::Moving {
                        wanted: wanted.clone(),
                        done,
                        total,
                        sending: true,
                    });
                }
            }
            if sender
                .send(&frame(END, request, &done.to_le_bytes()))
                .is_err()
            {
                break;
            }
            said = Instant::now();
            self.footage.told(News::Moving {
                wanted,
                done,
                total,
                sending: true,
            });
        }
        sender.close();
    }

    /// Read a link until it drops or says something that is not a frame.
    fn read_loop(self: &Arc<Self>, peer: u32, receiver: &mut Receiver) {
        while !self.stopped() {
            let Ok(bytes) = receiver.recv(FRAME_LIMIT) else {
                break;
            };
            let Some((&kind, rest)) = bytes.split_first() else {
                break;
            };
            if kind == STILL_HERE {
                continue;
            }
            let Some((request, rest)) = rest.split_first_chunk::<8>() else {
                break;
            };
            let request = u64::from_le_bytes(*request);
            let number = |rest: &[u8]| rest.first_chunk::<8>().map(|n| u64::from_le_bytes(*n));
            let kept = match kind {
                WANT => self.asked(peer, request, rest),
                SIZE => match (number(rest), rest.get(8..).and_then(number)) {
                    (Some(total), Some(from)) => self.sized(peer, request, total, from),
                    _ => false,
                },
                DATA => self.piece(peer, request, rest),
                END => number(rest).is_some_and(|total| self.whole(peer, request, total)),
                NO => {
                    let gone = self.taking.lock().remove(&request);
                    if let Some(gone) = gone.filter(|t| t.peer == peer) {
                        self.refuse(&gone.wanted);
                    }
                    true
                }
                _ => false,
            };
            if !kept {
                break;
            }
        }
    }

    /// The other end wants something: hand it to this link's maker.
    fn asked(&self, peer: u32, request: u64, rest: &[u8]) -> bool {
        let Some((have, rest)) = rest.split_first_chunk::<8>() else {
            return false;
        };
        let Some((start, text)) = rest.split_first_chunk::<32>() else {
            return false;
        };
        let Ok(wanted) = serde_json::from_slice::<Wanted>(text) else {
            return false;
        };
        let resume = (u64::from_le_bytes(*have), *start);
        let links = self.links.lock();
        let Some(link) = links.iter().find(|l| l.peer == peer) else {
            return false;
        };
        // Asked for more than a link holds at once, the rest is refused
        // rather than waited on: it can be asked for again.
        if link.make.try_send((request, resume, wanted)).is_err() {
            let _ = link.out.try_send(Out::Frame(frame(NO, request, &[])));
        }
        true
    }

    /// The file a request is answered with is `total` bytes, and is coming
    /// from byte `from`: nought, or as much as this machine said it had.
    fn sized(&self, peer: u32, request: u64, total: u64, from: u64) -> bool {
        let mut taking = self.taking.lock();
        let Some(taking) = taking.get_mut(&request).filter(|t| t.peer == peer) else {
            return true;
        };
        let path = part(&taking.path);
        if let Some(folder) = path.parent() {
            let _ = fs::create_dir_all(folder);
        }
        let opened = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path);
        let Ok(mut file) = opened else {
            return false;
        };
        if from > taking.done || from > total {
            return false;
        }
        if file.set_len(from).is_err() || file.seek(SeekFrom::End(0)).is_err() {
            return false;
        }
        taking.done = from;
        taking.total = total;
        taking.file = Some(file);
        true
    }

    fn piece(&self, peer: u32, request: u64, bytes: &[u8]) -> bool {
        self.limits.down.take(bytes.len(), &self.stop);
        let mut all = self.taking.lock();
        let Some(taking) = all.get_mut(&request).filter(|t| t.peer == peer) else {
            // Refused or forgotten since. What is on its way is let pass.
            return true;
        };
        let Some(file) = taking.file.as_mut() else {
            return false;
        };
        taking.done += bytes.len() as u64;
        if taking.done > taking.total || file.write_all(bytes).is_err() {
            return false;
        }
        if taking.told.elapsed() >= TELLING {
            taking.told = Instant::now();
            let news = News::Moving {
                wanted: taking.wanted.clone(),
                done: taking.done,
                total: taking.total,
                sending: false,
            };
            drop(all);
            self.footage.told(news);
        }
        true
    }

    /// A file has all arrived: put it in its place, say so, and on the host
    /// pass it on to whoever asked the host for it.
    fn whole(self: &Arc<Self>, peer: u32, request: u64, total: u64) -> bool {
        let taken = {
            let mut taking = self.taking.lock();
            match taking.get(&request) {
                Some(t) if t.peer == peer => taking.remove(&request),
                _ => return true,
            }
        };
        let Some(mut taken) = taken else {
            return true;
        };
        let landed = taken.done == total && taken.total == total && {
            let flushed = taken
                .file
                .take()
                .is_some_and(|file| file.sync_all().is_ok());
            let _ = fs::remove_file(&taken.path);
            flushed && fs::rename(part(&taken.path), &taken.path).is_ok()
        };
        if !landed {
            let _ = fs::remove_file(part(&taken.path));
            self.refuse(&taken.wanted);
            return true;
        }
        let owed: Vec<Owed> = {
            let mut all = self.owed.lock();
            let (ready, kept) = all.drain(..).partition(|o| o.wanted == taken.wanted);
            *all = kept;
            ready
        };
        for owed in owed {
            if let Some(out) = self.out(owed.peer) {
                let _ = out.try_send(Out::File {
                    request: owed.request,
                    wanted: owed.wanted,
                    path: taken.path.clone(),
                    resume: owed.resume,
                });
            }
        }
        self.footage.told(News::Arrived {
            wanted: taken.wanted,
            path: taken.path,
        });
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A limit is kept to: what is taken faster than it allows is waited
    /// for, and nought is no limit at all.
    #[test]
    fn a_limit_makes_a_taker_wait() {
        let (limits, stop) = (Limits::default(), AtomicBool::new(false));
        let start = Instant::now();
        for _ in 0..20 {
            limits.up.take(PIECE, &stop);
        }
        assert!(start.elapsed() < Duration::from_millis(200));

        // Forty pieces at twenty a second, with a second's worth to start.
        limits.set(20 * PIECE as u64, 0);
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        for _ in 0..40 {
            limits.up.take(PIECE, &stop);
        }
        assert!(start.elapsed() > Duration::from_millis(800));
        assert_eq!(limits.get(), (20 * PIECE as u64, 0));
    }
}
