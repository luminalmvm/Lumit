//! Shared projects: several people editing one project at the same time.
//!
//! One person hosts. Their Lumit puts every edit in order and owns the file.
//! Guests connect to it, are sent the document, and from then on each end
//! sends the edits it makes. Footage never travels: each machine finds its own
//! copy by fingerprint.
//!
//! Nothing here is run by Lumit's makers. The host listens on its own machine
//! and the guests reach it directly, over a LAN or a VPN.
//!
//! Threads: sharing runs its own and none is the UI thread. The host has one
//! that accepts, and a reader and a writer per guest. A guest has a reader and
//! a writer.

mod guest;
mod host;
mod kept;
mod local;
mod wire;

pub use guest::{join, resume, Guest, Joining, Resuming};
pub use host::{host, Host};
pub use kept::forget;
pub use lumit_core::shared::Conflict;

use lumit_core::CompTime;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::UdpSocket;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

/// The port a host listens on unless told otherwise. Fixed, so it can be
/// forwarded once on a router and stay forwarded.
pub const DEFAULT_PORT: u16 = 47856;

/// The most people in one shared project, the host included.
pub const MAX_PEOPLE: usize = 16;

/// What one person is looking at, for the others to see.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Presence {
    /// The composition they have open.
    pub comp: Option<Uuid>,
    /// The layers they have selected in it.
    pub layers: Vec<Uuid>,
    pub playhead: Option<CompTime>,
    /// Their pointer over the Viewer, in composition pixels.
    pub cursor: Option<(f64, f64)>,
}

impl Presence {
    /// Cut down to what is worth relaying, whoever sent it.
    fn tidied(mut self) -> Self {
        self.layers.truncate(256);
        self.cursor = self.cursor.filter(|(x, y)| x.is_finite() && y.is_finite());
        self
    }
}

/// One person in a shared project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    /// The host is 0. Guests are numbered as they join.
    pub id: u32,
    pub name: String,
    /// An index into the label palette, from 1, given out in joining order.
    pub colour: u8,
    pub presence: Presence,
}

/// Why a host turned a guest away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// The two run different versions of Lumit. Edits are the wire format, so
    /// both ends have to agree on every one of them.
    Version { host: String },
    /// The project already has [`MAX_PEOPLE`] in it.
    Full,
}

/// Why sharing stopped for a guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// The host stopped sharing.
    Closed,
    /// The host would not take this guest back.
    Refused(Refusal),
    /// The host's document is one this machine will not hold.
    Unsafe,
    /// The host took this guest out of the project.
    Removed,
}

/// What a shared project tells whoever is sharing it.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Who is here and what each is looking at, sent whole each time, and
    /// which of them is this end.
    People { me: u32, people: Vec<Person> },
    /// A guest has lost its host and is carrying on alone until it is back.
    Away,
    /// The host is back and the edits made meanwhile are merged. The document
    /// was replaced, `held` conflicts wait to be chosen between, and `refused`
    /// edits no longer applied.
    Back { held: usize, refused: usize },
    /// The invite a guest is looking for its host by leads to a different
    /// project. It goes on looking, and wants another invite.
    Elsewhere,
    /// Sharing is over for this guest. The document stays as it is.
    Ended(Ending),
}

/// Where those events go. Called from the share threads with no lock held.
pub type Events = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
    #[error("the encrypted channel failed: {0}")]
    Channel(#[from] snow::Error),
    #[error("a message could not be read: {0}")]
    Json(#[from] serde_json::Error),
    #[error("that is not a Lumit invite")]
    BadInvite,
    #[error("the host could not be reached")]
    Unreachable,
    #[error("the host turned this guest away: {0:?}")]
    Refused(Refusal),
    #[error("a message of {size} bytes where at most {limit} is read")]
    TooLarge { size: u64, limit: u64 },
    #[error("the other end sent something out of turn")]
    OutOfTurn,
    #[error("the host's project is one this machine will not hold")]
    Unsafe,
    #[error("no randomness for the invite: {0}")]
    NoRandomness(String),
}

/// What a guest needs to reach a host: where it is, and the secret that lets
/// it in and keys the channel. Written `address:port/key`, with an IPv6
/// address in square brackets.
///
/// The key is 256 random bits, so there is nothing to guess and no need for a
/// password exchange. Whoever holds an invite can join and edit, until the
/// host stops sharing or takes someone out, which replaces it for everyone
/// still there. A host that only closes the project keeps the key, so the
/// same invite works when it shares that project again.
#[derive(Clone, PartialEq, Eq)]
pub struct Invite {
    /// `host:port`, where host is an address or a name.
    pub address: String,
    pub key: [u8; 32],
}

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, key_text(&self.key))
    }
}

/// An invite's secret as it is written, for a host to keep and share the
/// same project by again.
#[must_use]
pub fn key_text(key: &[u8; 32]) -> String {
    hex::encode(key)
}

/// The secret [`key_text`] wrote, or `None` for anything else.
#[must_use]
pub fn key_from(text: &str) -> Option<[u8; 32]> {
    let mut key = [0u8; 32];
    hex::decode_to_slice(text.trim(), &mut key).ok()?;
    Some(key)
}

impl FromStr for Invite {
    type Err = ShareError;

    fn from_str(text: &str) -> Result<Self, ShareError> {
        let (address, key) = text.trim().rsplit_once('/').ok_or(ShareError::BadInvite)?;
        let key = key_from(key).ok_or(ShareError::BadInvite)?;
        if address.is_empty() {
            return Err(ShareError::BadInvite);
        }
        let address = address.to_owned();
        Ok(Invite { address, key })
    }
}

/// This machine's address on its network, for the host to hand out. A guess:
/// the address the default route leaves from, which is the LAN one. A host
/// reached over a VPN or a forwarded port types that address instead.
///
/// Nothing is sent. Connecting a UDP socket only asks the system which
/// interface it would use.
#[must_use]
pub fn local_address() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("192.0.2.1:9")?;
            socket.local_addr()
        })
        .map_or_else(|_| "127.0.0.1".to_owned(), |addr| addr.ip().to_string())
}

/// One end of a shared project, whichever it is.
pub enum Sharing {
    Host(Host),
    Guest(Guest),
}

impl Sharing {
    /// Everyone in the project, this end included.
    #[must_use]
    pub fn people(&self) -> Vec<Person> {
        match self {
            Sharing::Host(host) => host.people(),
            Sharing::Guest(guest) => guest.people(),
        }
    }

    /// Which of [`Self::people`] is this end.
    #[must_use]
    pub fn me(&self) -> u32 {
        match self {
            Sharing::Host(_) => 0,
            Sharing::Guest(guest) => guest.me(),
        }
    }

    /// Tell the others what this end is looking at. Latest wins.
    pub fn set_presence(&self, presence: Presence) {
        match self {
            Sharing::Host(host) => host.set_presence(presence),
            Sharing::Guest(guest) => guest.set_presence(presence),
        }
    }

    /// Stop sharing for everyone, or leave as a guest. The document stays as
    /// it is. Dropping a host instead leaves its guests waiting for it.
    pub fn stop(&self) {
        match self {
            Sharing::Host(host) => host.stop(),
            Sharing::Guest(guest) => guest.stop(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::{LinearColour, ProjectItem, SolidDef};
    use lumit_core::{Document, DocumentStore, Op};
    use parking_lot::Mutex;
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn quiet() -> Events {
        Arc::new(|_| {})
    }

    fn add_solid(store: &DocumentStore, name: &str) -> Uuid {
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

    fn rename(store: &DocumentStore, id: Uuid, name: &str) {
        let name = name.into();
        store.commit(Op::RenameItem { id, name }).unwrap();
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(20), "{what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A way to the host on `port` that can stop passing on what the host
    /// says. The first `deaf` connections made through it hear nothing more,
    /// as one that dies with answers still on their way does. Answers the
    /// port it listens on.
    fn relay(port: u16, deaf: Arc<AtomicUsize>) -> u16 {
        let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
        let here = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (nth, guest) in listener.incoming().flatten().enumerate() {
                let Ok(host) = TcpStream::connect((LOOPBACK, port)) else {
                    continue;
                };
                let pump = |mut from: TcpStream, mut to: TcpStream, deaf: Arc<AtomicUsize>| {
                    std::thread::spawn(move || {
                        let mut bytes = [0u8; 4096];
                        while let Ok(n @ 1..) = from.read(&mut bytes) {
                            let heard = deaf.load(Ordering::Relaxed) <= nth;
                            if heard && to.write_all(&bytes[..n]).is_err() {
                                break;
                            }
                        }
                        let _ = to.shutdown(Shutdown::Both);
                    });
                };
                let (to_host, to_guest) = (host.try_clone().unwrap(), guest.try_clone().unwrap());
                pump(guest, to_host, Arc::new(AtomicUsize::new(0)));
                pump(host, to_guest, deaf.clone());
            }
        });
        here
    }

    /// The whole road over real sockets: a guest is sent the document, both
    /// ends edit at once, the same item included, and they end on the same
    /// document. Presence crosses, a dropped connection loses nothing and
    /// makes no edit twice, and the guest hears the host stop.
    #[test]
    fn a_host_and_a_guest_end_on_the_same_document() {
        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let shared = add_solid(&hosted, "before");
        let host = host(hosted.clone(), "Host", LOOPBACK, 0, None, None, quiet()).unwrap();
        let deaf = Arc::new(AtomicUsize::new(0));
        let through = relay(host.port(), deaf.clone());
        let invite: Invite = host.invite("127.0.0.1").to_string().parse().unwrap();
        let invite = Invite {
            address: format!("127.0.0.1:{through}"),
            ..invite
        };

        let (document, joining) = join(invite.clone(), "Guest", None).unwrap();
        assert_eq!(document.items, hosted.snapshot().items);
        let joined = Arc::new(DocumentStore::new(document));
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let events: Events = Arc::new(move |event| sink.lock().push(event));
        let guest = joining.start(joined.clone(), events).unwrap();

        let hosts = add_solid(&hosted, "the host's");
        let guests = add_solid(&joined, "the guest's");
        rename(&hosted, shared, "the host says");
        rename(&joined, shared, "the guest says");
        until("both ends hold both new items and agree", || {
            let (h, g) = (hosted.snapshot(), joined.snapshot());
            h.item(guests).is_some() && g.item(hosts).is_some() && h.items == g.items
        });

        until("the host sees its guest", || host.people().len() == 2);
        let comp = Some(Uuid::now_v7());
        guest.set_presence(Presence {
            comp,
            ..Presence::default()
        });
        until("the host sees where its guest is", || {
            host.people().iter().any(|p| p.presence.comp == comp)
        });

        // Someone else joins and is taken out. The invite they hold lets
        // nobody in after that, and the guest still here is sent the new one,
        // which is what it finds the host by from here on.
        let (document, joining) = join(invite.clone(), "Other", None).unwrap();
        let theirs = Arc::new(DocumentStore::new(document));
        let other = joining.start(theirs, quiet()).unwrap();
        until("the host sees both guests", || host.people().len() == 3);
        host.remove(other.me());
        assert!(join(invite, "Other", None).is_err());
        assert_eq!(host.people().len(), 2);
        until("the guest still here has the new invite", || {
            guest.key() == host.key()
        });

        // The connection drops, with an edit on it that the host took and
        // never got to answer. The host says so when the guest is back, and
        // the edit is not made a second time, which here would be refused:
        // the item is already gone. An edit made on each side of the gap is
        // on both ends once the guest has found its host again.
        heard.lock().clear();
        deaf.store(1, Ordering::Relaxed);
        joined.commit(Op::RemoveItem { id: hosts }).unwrap();
        until("the host has taken the edit", || {
            hosted.snapshot().item(hosts).is_none()
        });
        guest.cut();
        let apart = add_solid(&joined, "made while away");
        let meanwhile = add_solid(&hosted, "made meanwhile");
        until("the guest is back with nothing lost", || {
            let (h, g) = (hosted.snapshot(), joined.snapshot());
            h.item(apart).is_some() && g.item(meanwhile).is_some() && h.items == g.items
        });
        let back = Event::Back {
            held: 0,
            refused: 0,
        };
        until("nothing was merged twice", || heard.lock().contains(&back));

        // The host goes without saying it is over, as a closed project or a
        // crash does, and the guest works on. The same project shared again
        // by the same key on the same port is found, and what the guest did
        // meanwhile is on both ends.
        heard.lock().clear();
        let (key, port) = (host.key(), host.port());
        drop(host);
        until("the guest misses its host", || {
            heard.lock().contains(&Event::Away)
        });
        let orphaned = add_solid(&joined, "made with no host");
        let again = crate::host(
            hosted.clone(),
            "Host",
            LOOPBACK,
            port,
            Some(key),
            None,
            quiet(),
        );
        let host = again.unwrap();
        until("the guest is back with the host that restarted", || {
            let (h, g) = (hosted.snapshot(), joined.snapshot());
            h.item(orphaned).is_some() && h.items == g.items
        });

        host.stop();
        until("the guest hears the host stop", || {
            heard.lock().contains(&Event::Ended(Ending::Closed))
        });
    }

    /// Closing Lumit loses nobody's work. A host that closed without saving
    /// has every edit since its last save back when it shares again, its
    /// guest's included. A guest that closed while its host was away opens
    /// its copy as it left it, and that finds the host and merges. What it
    /// chose not to save is not in it, and a conflict it closed on is asked
    /// again.
    #[test]
    fn a_host_and_a_guest_that_both_closed_carry_on_where_they_left_off() {
        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let both = add_solid(&hosted, "saved");
        let host = host(hosted.clone(), "Host", LOOPBACK, 0, None, None, quiet()).unwrap();
        let invite: Invite = host.invite("127.0.0.1").to_string().parse().unwrap();
        let (document, joining) = join(invite, "Guest", None).unwrap();
        let joined = Arc::new(DocumentStore::new(document));
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let events: Events = Arc::new(move |event| sink.lock().push(event));
        let guest = joining.start(joined.clone(), events).unwrap();

        // An edit, a save, and then one from each end the file never sees.
        let before = add_solid(&hosted, "made before the save");
        let (saved, _, mark) = host.saving();
        let file = Document::clone(&saved);
        host.saved(mark);
        let hosts = add_solid(&hosted, "the host's, not saved");
        let guests = add_solid(&joined, "the guest's, not saved by the host");
        until("each end has the other's edit", || {
            let (h, g) = (hosted.snapshot(), joined.snapshot());
            h.item(guests).is_some() && g.item(hosts).is_some()
        });

        // The host closes without saving. The guest works on, and closes too.
        let (key, port) = (host.key(), host.port());
        drop(host);
        until("the guest misses its host", || {
            heard.lock().contains(&Event::Away)
        });
        let apart = add_solid(&joined, "made with no host");
        // More edits than a connection queues, which all cross at once when
        // the host is found.
        for n in 0..1100 {
            rename(&joined, before, &format!("renamed {n}"));
        }
        rename(&joined, before, "renamed with no host");
        rename(&joined, both, "the guest's name");
        // It saves, makes one more edit, and closes without saving that.
        let (copy, _, mark) = guest.saving().expect("the host is away");
        let copy = Document::clone(&copy);
        guest.saved(mark);
        let unsaved = add_solid(&joined, "made after the save");
        guest.discard();
        drop(guest);
        drop(joined);

        // The host opens the file it saved and shares it again.
        let hosted = Arc::new(DocumentStore::new(file));
        let again = crate::host(
            hosted.clone(),
            "Host",
            LOOPBACK,
            port,
            Some(key),
            None,
            quiet(),
        );
        let host = again.unwrap();
        assert_eq!(host.restored(), 2, "the two edits made after the save");
        let back = hosted.snapshot();
        assert!(back.item(hosts).is_some() && back.item(guests).is_some());
        rename(&hosted, both, "the host's name");

        // The guest opens its copy: what it did with no host and saved is in
        // it, and goes to the host once that is found.
        let nowhere = std::path::Path::new("");
        let kept = resume(&mut copy.clone(), nowhere);
        let (document, resuming) = kept.expect("the guest's edits were kept");
        assert!(document.item(apart).is_some());
        assert!(document.item(unsaved).is_none(), "it was not saved");
        let joined = Arc::new(DocumentStore::new(document));
        let guest = resuming.start(joined.clone(), quiet()).unwrap();
        until("the host has what its guest did while away", || {
            let (h, g) = (hosted.snapshot(), joined.snapshot());
            let renamed = h.item(before).map(ProjectItem::name) == Some("renamed with no host");
            h.item(apart).is_some() && renamed && h.items == g.items
        });
        until("nothing is kept once it is merged", || {
            resume(&mut copy.clone(), nowhere).is_none()
        });

        // Both renamed one item, so the guest's name for it was held back.
        // It closes without choosing, and its copy opened again still asks.
        until("the merge held the guest's name back", || {
            guest.conflicts().len() == 1
        });
        let copy = Document::clone(&joined.snapshot());
        drop(guest);
        drop(joined);
        let kept = resume(&mut copy.clone(), nowhere);
        let (document, resuming) = kept.expect("the conflict was kept");
        let joined = Arc::new(DocumentStore::new(document));
        let guest = resuming.start(joined, quiet()).unwrap();
        assert_eq!(guest.conflicts().len(), 1);
        guest.resolve(0, true);
        until("the host has the name its guest chose to keep", || {
            hosted.snapshot().item(both).map(ProjectItem::name) == Some("the guest's name")
        });

        // A host that stops for good keeps nothing to put back either.
        host.stop();
        drop(guest);
        let afresh = Arc::new(DocumentStore::new(Document::clone(&hosted.snapshot())));
        let host = crate::host(afresh, "Host", LOOPBACK, 0, Some(key), None, quiet()).unwrap();
        assert_eq!(host.restored(), 0);
    }

    /// The invite's secret is what lets a guest in. One bit out and the
    /// handshake fails before the host has said anything.
    #[test]
    fn a_guest_with_the_wrong_key_is_not_let_in() {
        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let host = host(hosted, "Host", LOOPBACK, 0, None, None, quiet()).unwrap();
        let mut invite = host.invite("127.0.0.1");
        invite.key[0] ^= 1;
        assert!(join(invite, "Guest", None).is_err());
    }
}
