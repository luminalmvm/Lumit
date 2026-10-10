//! Shared projects: several people editing one project at the same time.
//!
//! One person hosts. Their Lumit puts every edit in order and owns the file.
//! Guests connect to it, are sent the document, and from then on each end
//! sends the edits it makes. Footage never travels: each machine finds its own
//! copy by fingerprint.
//!
//! Nothing here is run by Lumit's makers. The host listens on its own machine
//! and the guests reach it directly: on its network, through its router, or
//! over a VPN. Where none of those lets a guest in, both can meet at a relay
//! one of them has the address of, which passes on what they say without
//! being able to read it.
//!
//! Threads: sharing runs its own and none is the UI thread. The host has one
//! that accepts, a reader and a writer per guest, one that keeps its
//! router's port open when it was asked to, and one that keeps its room at a
//! relay when it was given one. A guest has a reader and a writer, and for a
//! moment one for each address it tries its host at.

mod bulk;
mod guest;
mod host;
mod invite;
mod kept;
mod local;
mod reach;
mod wire;

pub use bulk::{Footage, Held, Limits, News, Wanted};
pub use guest::{join, resume, Guest, Joining, Resuming};
pub use host::{host, Host};
pub use invite::{key_from, key_text, lock_of, Invite, LINK, LINK_SCHEME, MAX_ADDRESSES};
pub use kept::forget;
pub use lumit_core::shared::Conflict;

use lumit_core::CompTime;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, UdpSocket};
use std::sync::Arc;
use uuid::Uuid;

/// The port a host listens on unless told otherwise. Fixed, so it can be
/// forwarded once on a router and stay forwarded.
pub const DEFAULT_PORT: u16 = 47856;

/// The port a relay listens on unless its owner picked another.
pub const RELAY_PORT: u16 = lumit_relay::DEFAULT_PORT;

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
    /// The property rows they have selected in the Timeline, and the
    /// keyframes, each by the name the interface knows it by. Nothing here
    /// reads them: they are only handed to the other interfaces to match
    /// against their own rows.
    #[serde(default)]
    pub properties: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
}

/// The most property rows and keyframes one person's selection is relayed
/// with, and the longest name any of them goes by.
const MARKED_PROPERTIES: usize = 256;
const MARKED_KEYS: usize = 2048;
const MARK_LENGTH: usize = 256;

impl Presence {
    /// Cut down to what is worth relaying, whoever sent it.
    fn tidied(mut self) -> Self {
        self.layers.truncate(256);
        self.cursor = self.cursor.filter(|(x, y)| x.is_finite() && y.is_finite());
        self.properties.retain(|name| name.len() <= MARK_LENGTH);
        self.properties.truncate(MARKED_PROPERTIES);
        self.keys.retain(|name| name.len() <= MARK_LENGTH);
        self.keys.truncate(MARKED_KEYS);
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

/// Whether people outside the host's network can reach it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Reach {
    /// The router was not asked. Only people on the host's own network, or
    /// on a VPN with it, or through a port forwarded by hand, can join.
    #[default]
    Off,
    /// The router is being asked.
    Asking,
    /// The router sends the port to this machine. `address` is the one it
    /// has on the internet, which is what goes in an invite for someone
    /// outside.
    Open { address: String },
    /// No router answered, or the one that did would not open the port.
    Refused,
    /// The router is not on the internet itself: it sits behind another, or
    /// behind an address its provider shares between customers. Opening its
    /// port would reach nobody.
    Behind,
}

/// Whether a host has a room at a relay, for guests no address of its own
/// lets in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Relayed {
    /// It was given no relay.
    #[default]
    Off,
    /// The relay is being asked.
    Asking,
    /// The room is open, and an invite made now leads to it.
    Open,
    /// The relay did not answer, or would not open a room. It is asked again
    /// every few seconds.
    Unreachable,
}

/// The name of the room a host with this invite keeps at a relay. Both ends
/// work it out from the secret, and the relay cannot work the secret out
/// from it.
pub(crate) fn room(key: &[u8; 32]) -> String {
    let name = blake3::derive_key("lumit-share 2026 relay room", key);
    hex::encode(&name[..16])
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
    /// For a host: what came of asking its router to let people outside
    /// the network in.
    Reach(Reach),
    /// For a host: what came of asking a relay for a room.
    Relayed(Relayed),
    /// Another person's Lumit said something to this one that is not an
    /// edit. `from` is who, and `body` is theirs to have written and this
    /// end's to judge.
    Note { from: u32, body: serde_json::Value },
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
    #[error("that invite needs its password")]
    Locked,
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

/// The address this machine has on the internet over IPv6, if it has one.
/// Every machine has its own there, with no router's address in front of it,
/// so it goes in an invite as one more way in. Whether the router lets a
/// stranger through to it is the router's business. Nothing is sent.
#[must_use]
pub fn global_address() -> Option<String> {
    let socket = UdpSocket::bind("[::]:0").ok()?;
    socket.connect("[2001:db8::1]:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        // 2000::/3 is what is routed between networks.
        IpAddr::V6(ip) if ip.segments()[0] & 0xe000 == 0x2000 => Some(ip.to_string()),
        _ => None,
    }
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

    /// Send and take footage, which is `footage`'s to make and keep, no
    /// faster than `limits`. Until this is called none crosses.
    pub fn carry_footage(&self, footage: Arc<dyn Footage>, limits: Arc<Limits>) {
        match self {
            Sharing::Host(host) => host.carry_footage(footage, limits),
            Sharing::Guest(guest) => guest.carry_footage(footage, limits),
        }
    }

    /// Say which footage items this machine has the original of. The whole
    /// list each time it changes.
    pub fn set_holds(&self, items: Vec<Held>) {
        match self {
            Sharing::Host(host) => host.set_holds(items),
            Sharing::Guest(guest) => guest.set_holds(items),
        }
    }

    /// Who has the original of the footage item `item`, by their number,
    /// and how big it is on their disk.
    #[must_use]
    pub fn holders(&self, item: Uuid) -> Vec<(u32, u64)> {
        match self {
            Sharing::Host(host) => host.holders(item),
            Sharing::Guest(guest) => guest.holders(item),
        }
    }

    /// Ask whoever has it for `wanted`. What comes of it is told to the
    /// [`Footage`] this end carries. Asked once however often it is called.
    pub fn want(&self, wanted: Wanted) {
        match self {
            Sharing::Host(host) => host.want(wanted),
            Sharing::Guest(guest) => guest.want(wanted),
        }
    }

    /// Say `body` to the person numbered `to`. It reaches them as an
    /// [`Event::Note`], or not at all if they have gone.
    pub fn note(&self, to: u32, body: serde_json::Value) {
        match self {
            Sharing::Host(host) => host.note(to, body),
            Sharing::Guest(guest) => guest.note(to, body),
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
        let host = host(
            hosted.clone(),
            "Host",
            LOOPBACK,
            0,
            None,
            None,
            None,
            quiet(),
        )
        .unwrap();
        let deaf = Arc::new(AtomicUsize::new(0));
        let through = relay(host.port(), deaf.clone());
        let invite: Invite = host.invite("127.0.0.1").to_string().parse().unwrap();
        // With an address that leads nowhere in it too, as the host's
        // address on its own network does for a guest somewhere else.
        let invite = Invite {
            addresses: vec!["127.0.0.1:1".into(), format!("127.0.0.1:{through}")],
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
        let host = host(
            hosted.clone(),
            "Host",
            LOOPBACK,
            0,
            None,
            None,
            None,
            quiet(),
        )
        .unwrap();
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
        let host =
            crate::host(afresh, "Host", LOOPBACK, 0, Some(key), None, None, quiet()).unwrap();
        assert_eq!(host.restored(), 0);
    }

    /// A guest that no address of the host's lets in meets it at a relay,
    /// and the two end on the same document. Taking someone out replaces
    /// the invite, and the room with it: the old invite finds nobody at the
    /// relay and the new one finds the host.
    #[test]
    fn a_guest_reaches_its_host_through_a_relay() {
        let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
        let relay = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let serving = std::thread::spawn(move || {
            lumit_relay::serve(&[listener], lumit_relay::Limits::default(), &stopping)
        });

        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let host = host(
            hosted.clone(),
            "Host",
            LOOPBACK,
            0,
            None,
            None,
            None,
            quiet(),
        )
        .unwrap();
        host.relay_through(&relay);
        until("the host has its room", || host.relayed() == Relayed::Open);
        // With the relay as the only way: nothing in it leads to the host.
        let by_relay = || {
            let invite = Invite {
                addresses: Vec::new(),
                ..host.invite_anywhere(None)
            };
            invite.to_string().parse::<Invite>().unwrap()
        };
        let old = by_relay();
        assert_eq!(old.relays, [relay]);

        let (document, joining) = join(old.clone(), "Guest", None).unwrap();
        let joined = Arc::new(DocumentStore::new(document));
        let guest = joining.start(joined.clone(), quiet()).unwrap();
        let hosts = add_solid(&hosted, "the host's");
        let guests = add_solid(&joined, "the guest's");
        until("both ends hold both new items and agree", || {
            let (h, g) = (hosted.snapshot(), joined.snapshot());
            h.item(guests).is_some() && g.item(hosts).is_some() && h.items == g.items
        });

        host.remove(guest.me());
        until("the new invite finds the host at the relay", || {
            join(by_relay(), "Other", None).is_ok()
        });
        assert!(join(old, "Guest", None).is_err());

        host.stop();
        stop.store(true, Ordering::Relaxed);
        serving.join().unwrap().unwrap();
    }

    /// A machine's footage for the tests: the originals it was given, and
    /// a folder of its own that what it is sent lands in.
    struct Shelf {
        folder: std::path::PathBuf,
        originals: Vec<(Uuid, std::path::PathBuf)>,
    }

    impl Footage for Shelf {
        fn make(&self, wanted: &Wanted, _: &AtomicBool) -> Option<std::path::PathBuf> {
            let item = wanted.item()?;
            let mine = self.originals.iter().find(|(id, _)| *id == item);
            let sent = self.room(wanted).filter(|path| path.is_file());
            mine.map(|(_, path)| path.clone()).or(sent)
        }

        fn room(&self, wanted: &Wanted) -> Option<std::path::PathBuf> {
            Some(self.folder.join(format!("{}.bin", wanted.item()?)))
        }

        fn told(&self, _: News) {}
    }

    /// Footage one guest has reaches a guest that has not, by way of a host
    /// that has not either and keeps what passes through it. A file part of
    /// which is here already is carried on with, and one whose part here is
    /// not how the file starts is begun again.
    #[test]
    fn footage_one_guest_has_reaches_another_through_the_host() {
        let root = std::env::temp_dir().join(format!("lumit-bulk-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shelf = |name: &str, originals: Vec<(Uuid, std::path::PathBuf)>| {
            let folder = root.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            Arc::new(Shelf { folder, originals })
        };
        let bytes = |seed: u8| -> Vec<u8> {
            (0..300_000u32)
                .map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed)
                .collect()
        };
        std::fs::create_dir_all(&root).unwrap();
        let (clip, other) = (Uuid::now_v7(), Uuid::now_v7());
        let (clip_file, other_file) = (root.join("clip.mov"), root.join("other.mov"));
        std::fs::write(&clip_file, bytes(1)).unwrap();
        std::fs::write(&other_file, bytes(2)).unwrap();

        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let host = host(hosted, "Host", LOOPBACK, 0, None, None, None, quiet()).unwrap();
        let hosts = shelf("host", Vec::new());
        host.carry_footage(hosts.clone(), Arc::new(Limits::default()));
        let invite = || host.invite("127.0.0.1");
        let guest = |name: &str, shelf: Arc<Shelf>| {
            let (document, joining) = join(invite(), name, None).unwrap();
            let store = Arc::new(DocumentStore::new(document));
            let guest = joining.start(store, quiet()).unwrap();
            guest.carry_footage(shelf, Arc::new(Limits::default()));
            guest
        };
        let has = guest(
            "Has",
            shelf("has", vec![(clip, clip_file), (other, other_file)]),
        );
        let lacks_shelf = shelf("lacks", Vec::new());
        let lacks = guest("Lacks", lacks_shelf.clone());
        let held = |item| Held {
            item,
            bytes: 300_000,
        };
        has.set_holds(vec![held(clip), held(other)]);
        until("everyone knows who has the footage", || {
            let theirs = [(has.me(), 300_000)];
            lacks.holders(clip) == theirs && host.holders(other) == theirs
        });

        // Part of one is here already, and part of the other is not it.
        let part = |name: Uuid| lacks_shelf.folder.join(format!("{name}.bin.part"));
        std::fs::write(part(clip), &bytes(1)[..100_000]).unwrap();
        std::fs::write(part(other), &bytes(9)[..100_000]).unwrap();
        lacks.want(Wanted::StandIn { item: clip });
        lacks.want(Wanted::StandIn { item: other });
        let arrived = |shelf: &Shelf, item: Uuid, seed: u8| {
            let path = shelf.folder.join(format!("{item}.bin"));
            std::fs::read(path).is_ok_and(|read| read == bytes(seed))
        };
        until("both files reach the guest that lacked them", || {
            arrived(&lacks_shelf, clip, 1) && arrived(&lacks_shelf, other, 2)
        });
        assert!(arrived(&hosts, clip, 1) && arrived(&hosts, other, 2));

        host.stop();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The invite's secret is what lets a guest in. One bit out and the
    /// handshake fails before the host has said anything.
    #[test]
    fn a_guest_with_the_wrong_key_is_not_let_in() {
        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let host = host(hosted, "Host", LOOPBACK, 0, None, None, None, quiet()).unwrap();
        let mut invite = host.invite("127.0.0.1");
        invite.key[0] ^= 1;
        assert!(join(invite, "Guest", None).is_err());
    }

    /// With a password set, the link is not enough: a guest has to give the
    /// password too, and the wrong one gets no answer. The password outlives
    /// the invite, so after someone is taken out the new link opens with the
    /// same one, and the guest still here is sent a key that finds the host.
    #[test]
    fn a_password_is_asked_for_as_well_as_the_link() {
        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let lock = Some(lock_of("correct horse"));
        let host = host(hosted, "Host", LOOPBACK, 0, None, lock, None, quiet()).unwrap();
        let link = || {
            let written = host.invite("127.0.0.1").to_string();
            written.parse::<Invite>().unwrap()
        };
        assert!(link().locked);
        assert!(matches!(
            join(link(), "Guest", None),
            Err(ShareError::Locked)
        ));
        assert!(join(link().unlocked("wrong"), "Guest", None).is_err());
        // The link's own secret is not the key either.
        let bare = Invite {
            locked: false,
            ..link()
        };
        assert!(join(bare, "Guest", None).is_err());

        let (document, joining) = join(link().unlocked("correct horse"), "Guest", None).unwrap();
        let joined = Arc::new(DocumentStore::new(document));
        let guest = joining.start(joined, quiet()).unwrap();
        let (_, other) = join(link().unlocked("correct horse"), "Other", None).unwrap();
        let theirs = Arc::new(DocumentStore::new(Document::new()));
        let other = other.start(theirs, quiet()).unwrap();
        until("the host sees both guests", || host.people().len() == 3);
        let old = link();
        host.remove(other.me());
        assert!(join(old.unlocked("correct horse"), "Other", None).is_err());
        let new = link().unlocked("correct horse");
        until("the guest still here has the new key", || {
            guest.key() == new.key
        });
        assert!(join(new, "Other", None).is_ok());
    }
}
