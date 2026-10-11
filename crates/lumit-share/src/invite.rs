//! The invite a host hands out, and the link it is written as.
//!
//! Plain data and text. Nothing here touches the network.

use crate::ShareError;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

/// What a link starts with. A page that is only ever read: everything after
/// the `#` stays in the browser that opened it, which hands it to Lumit, so
/// the secret reaches no server.
pub const LINK: &str = "https://lumitlab.com/join#";

/// The same link as the system hands it to Lumit once the page is clicked
/// through.
pub const LINK_SCHEME: &str = "lumit://join/";

/// The most addresses one invite carries, relays included. A guest tries
/// them all at once.
pub const MAX_ADDRESSES: usize = 8;

/// Raised when what a link packs changes shape.
const PACKED: u8 = 1;

/// What a link starts with instead when the host set a password. 2 was a
/// password with no salt, which no build reads any more.
const PACKED_LOCKED: u8 = 3;

/// What a guest's own copy of such a link starts with once the password has
/// been given: the key the channel opens with, and the link's own secret.
const PACKED_OPENED: u8 = 4;

/// What Argon2id spends on a password: 64 MiB of memory, gone over three
/// times, on one thread. Trying a list of them costs that for every guess.
const MEMORY_KIB: u32 = 64 << 10;
const PASSES: u32 = 3;
const LANES: u32 = 1;

/// What a host keeps of a password, and never the password: what it comes to
/// with a salt of its own, so the work of trying a list of passwords against
/// one host is no use against another.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Lock {
    salt: [u8; 16],
    key: [u8; 32],
}

impl Lock {
    /// The lock for a password being set now.
    pub fn new(password: &str) -> Result<Lock, ShareError> {
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).map_err(|e| ShareError::NoRandomness(e.to_string()))?;
        Lock::with(password, salt).ok_or(ShareError::Password)
    }

    /// Slow on purpose. `None` when the password will not hash, which with
    /// the costs above is one too long to be one.
    fn with(password: &str, salt: [u8; 16]) -> Option<Lock> {
        let params = argon2::Params::new(MEMORY_KIB, PASSES, LANES, Some(32)).ok()?;
        let hasher =
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut key = [0u8; 32];
        hasher
            .hash_password_into(password.as_bytes(), &salt, &mut key)
            .ok()?;
        Some(Lock { salt, key })
    }

    /// As it is written, for a host to keep beside its invite's secret.
    #[must_use]
    pub fn text(&self) -> String {
        format!("{}{}", hex::encode(self.salt), hex::encode(self.key))
    }

    /// The lock [`Self::text`] wrote, or `None` for anything else.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Lock> {
        let mut bytes = [0u8; 48];
        hex::decode_to_slice(text.trim(), &mut bytes).ok()?;
        let (salt, key) = bytes.split_at(16);
        Some(Lock {
            salt: salt.try_into().ok()?,
            key: key.try_into().ok()?,
        })
    }

    pub(crate) fn salt(&self) -> [u8; 16] {
        self.salt
    }
}

/// What a guest needs to reach a host: where it may be, and the secret that
/// lets it in and keys the channel.
///
/// Written as a link, `https://lumitlab.com/join#…`, whose last part packs
/// the lot. Also read as `address:port/key`, with an IPv6 address in square
/// brackets, which is how it was written before and is still the plainest
/// way to type one by hand.
///
/// The key is 256 random bits, so there is nothing to guess and no need for a
/// password exchange. Whoever holds an invite can join and edit, until the
/// host stops sharing or takes someone out, which replaces it for everyone
/// still there. A host that only closes the project keeps the key, so the
/// same invite works when it shares that project again.
#[derive(Clone, PartialEq, Eq)]
pub struct Invite {
    /// Each `host:port` the host may be reached at, where host is an address
    /// or a name: on its own network, through its router, over a VPN. Only
    /// the one that holds the key is ever spoken to, so a wrong one costs
    /// nothing.
    pub addresses: Vec<String>,
    /// Each `host:port` of a relay the host keeps a room at, for a guest no
    /// address above lets in. See `lumit-relay`.
    pub relays: Vec<String>,
    /// The key the channel is opened with. For a [`Self::locked`] invite it
    /// is not that yet, and [`Self::unlocked`] makes it so.
    pub key: [u8; 32],
    /// The link's own secret. The same as `key` unless the host set a
    /// password. It is what the host's room at a relay is named from, and
    /// never the key the password has gone into: a room's name is said in
    /// the clear, and a name made from the password as well would let
    /// anyone holding a leaked link try passwords against the relay.
    pub link: [u8; 32],
    /// The host set a password, which the guest has to give as well as
    /// holding the link. A link that leaks is then not enough to join by.
    pub locked: bool,
    /// What the host's password is hashed with, for a locked invite.
    pub salt: [u8; 16],
}

/// The key the channel is opened with when the host set a password: the
/// link's secret and the password's, neither enough without the other.
#[must_use]
pub(crate) fn locked_key(link: &[u8; 32], lock: &Lock) -> [u8; 32] {
    *blake3::keyed_hash(link, &lock.key).as_bytes()
}

impl Invite {
    /// An invite with no password, whose one secret is `key`.
    #[must_use]
    pub fn open(addresses: Vec<String>, relays: Vec<String>, key: [u8; 32]) -> Invite {
        Invite {
            addresses,
            relays,
            key,
            link: key,
            locked: false,
            salt: [0; 16],
        }
    }

    /// This invite with its password given, ready to join by. One that
    /// needed none comes back as it was. A wrong password is not found out
    /// here: the host does not answer to it. One that will not hash leaves
    /// the invite locked.
    #[must_use]
    pub fn unlocked(mut self, password: &str) -> Invite {
        let lock = self.locked.then(|| Lock::with(password, self.salt));
        if let Some(lock) = lock.flatten() {
            self.key = locked_key(&self.link, &lock);
            self.locked = false;
            self.salt = [0; 16];
        }
        self
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

/// The 64 letters a link's last part is written in. No padding, and nothing
/// a chat program would cut a link short at.
const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn to_letters(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let mut word = [0u8; 3];
        word[..group.len()].copy_from_slice(group);
        let word = u32::from_be_bytes([0, word[0], word[1], word[2]]);
        for nth in 0..=group.len() {
            let six = (word >> (18 - 6 * nth)) & 63;
            text.push(char::from(LETTERS[six as usize]));
        }
    }
    text
}

fn from_letters(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3 + 2);
    for group in text.as_bytes().chunks(4) {
        // One letter alone carries less than a byte.
        if group.len() == 1 {
            return None;
        }
        let mut word = 0u32;
        for (nth, letter) in group.iter().enumerate() {
            let six = LETTERS.iter().position(|l| l == letter)?;
            word |= (six as u32) << (18 - 6 * nth);
        }
        let [_, a, b, c] = word.to_be_bytes();
        bytes.extend_from_slice(&[a, b, c][..group.len() - 1]);
    }
    Some(bytes)
}

/// Added to the kind of an address that is a relay's.
const RELAY: u8 = 0x10;

/// An address as a link packs it: a number for which kind, the address, and
/// the port. `None` for one that is not `host:port`.
fn pack(address: &str, relay: bool, into: &mut Vec<u8>) -> Option<()> {
    let relay = if relay { RELAY } else { 0 };
    let port = if let Ok(at) = address.parse::<SocketAddr>() {
        match at.ip() {
            IpAddr::V4(ip) => {
                into.push(4 | relay);
                into.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                into.push(6 | relay);
                into.extend_from_slice(&ip.octets());
            }
        }
        at.port()
    } else {
        let (name, port) = address.rsplit_once(':')?;
        // Only what reading the link back will take. One name it would not
        // is left out here, where otherwise it would spoil the whole link
        // for everyone it was sent to.
        if !name.chars().all(plain) {
            return None;
        }
        let length = u8::try_from(name.len()).ok().filter(|n| *n > 0)?;
        let port = port.parse().ok()?;
        into.push(relay);
        into.push(length);
        into.extend_from_slice(name.as_bytes());
        port
    };
    into.extend_from_slice(&port.to_be_bytes());
    Some(())
}

/// Whether `c` can be part of a name that is looked up.
fn plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-'
}

/// Take `n` bytes off the front of `bytes`.
fn take<'a>(bytes: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (front, rest) = bytes.split_at_checked(n)?;
    *bytes = rest;
    Some(front)
}

fn unpack(mut bytes: &[u8]) -> Option<Invite> {
    let kind = take(&mut bytes, 1)?[0];
    if ![PACKED, PACKED_LOCKED, PACKED_OPENED].contains(&kind) {
        return None;
    }
    let locked = kind == PACKED_LOCKED;
    let key: [u8; 32] = take(&mut bytes, 32)?.try_into().ok()?;
    let salt = match locked {
        true => take(&mut bytes, 16)?.try_into().ok()?,
        false => [0; 16],
    };
    let link = match kind {
        PACKED_OPENED => take(&mut bytes, 32)?.try_into().ok()?,
        _ => key,
    };
    let (mut addresses, mut relays) = (Vec::new(), Vec::new());
    while !bytes.is_empty() && addresses.len() + relays.len() < MAX_ADDRESSES {
        let kind = take(&mut bytes, 1)?[0];
        let host = match kind & !RELAY {
            4 => {
                let ip: [u8; 4] = take(&mut bytes, 4)?.try_into().ok()?;
                IpAddr::from(ip).to_string()
            }
            6 => {
                let ip: [u8; 16] = take(&mut bytes, 16)?.try_into().ok()?;
                format!("[{}]", IpAddr::from(ip))
            }
            0 => {
                let length = usize::from(take(&mut bytes, 1)?[0]);
                let name = std::str::from_utf8(take(&mut bytes, length)?).ok()?;
                // A name is looked up, so it is only what a name can be.
                if name.is_empty() || !name.chars().all(plain) {
                    return None;
                }
                name.to_owned()
            }
            _ => return None,
        };
        let port = u16::from_be_bytes(take(&mut bytes, 2)?.try_into().ok()?);
        let list = if kind & RELAY == 0 {
            &mut addresses
        } else {
            &mut relays
        };
        list.push(format!("{host}:{port}"));
    }
    if addresses.is_empty() && relays.is_empty() {
        return None;
    }
    Some(Invite {
        addresses,
        relays,
        key,
        link,
        locked,
        salt,
    })
}

impl fmt::Display for Invite {
    /// The link. An address that is not `host:port` is left out of it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut packed = Vec::new();
        if self.locked {
            packed.push(PACKED_LOCKED);
            packed.extend_from_slice(&self.link);
            packed.extend_from_slice(&self.salt);
        } else if self.key != self.link {
            packed.push(PACKED_OPENED);
            packed.extend_from_slice(&self.key);
            packed.extend_from_slice(&self.link);
        } else {
            packed.push(PACKED);
            packed.extend_from_slice(&self.key);
        }
        let direct = self.addresses.iter().map(|address| (address, false));
        let relayed = self.relays.iter().map(|address| (address, true));
        for (address, relay) in direct.chain(relayed).take(MAX_ADDRESSES) {
            let before = packed.len();
            if pack(address, relay, &mut packed).is_none() {
                packed.truncate(before);
            }
        }
        write!(f, "{LINK}{}", to_letters(&packed))
    }
}

impl FromStr for Invite {
    type Err = ShareError;

    fn from_str(text: &str) -> Result<Self, ShareError> {
        let text = text.trim();
        // A link, however it came: off the page, as the system hands it over,
        // or its last part alone.
        let packed = match text.split_once('#') {
            Some((_, packed)) => Some(packed),
            None => text.strip_prefix(LINK_SCHEME),
        };
        if let Some(invite) = from_letters(packed.unwrap_or(text)).and_then(|b| unpack(&b)) {
            return Ok(invite);
        }
        let (address, key) = text.rsplit_once('/').ok_or(ShareError::BadInvite)?;
        let key = key_from(key).ok_or(ShareError::BadInvite)?;
        if address.is_empty() {
            return Err(ShareError::BadInvite);
        }
        Ok(Invite::open(vec![address.to_owned()], Vec::new(), key))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A link holds every way to the host and the secret, whichever of its
    /// forms comes back, and an invite typed the old way is still one.
    /// Anything else is refused rather than half read.
    #[test]
    fn an_invite_survives_being_written_as_a_link() {
        let invite = Invite {
            addresses: vec![
                "192.168.1.20:47856".into(),
                "203.0.113.7:47856".into(),
                "[2001:db8::7]:47856".into(),
                "editor.example.org:5000".into(),
            ],
            relays: vec!["relay.example.org:47857".into()],
            locked: true,
            salt: [9; 16],
            key: std::array::from_fn(|n| n as u8 * 7),
            link: std::array::from_fn(|n| n as u8 * 7),
        };
        let link = invite.to_string();
        let packed = link.strip_prefix(LINK).unwrap();
        assert!(packed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        for text in [
            link.clone(),
            format!("  {LINK_SCHEME}{packed}\n"),
            packed.to_owned(),
        ] {
            assert!(text.parse::<Invite>().unwrap() == invite, "{text}");
        }

        let typed = format!("[::1]:47856/{}", key_text(&invite.key));
        let typed: Invite = typed.parse().unwrap();
        assert_eq!(typed.addresses, ["[::1]:47856"]);
        assert!(typed.key == invite.key);

        let cut = &link[..link.len() - 3];
        let named = format!(
            "{LINK}{}",
            to_letters(&[&[PACKED; 33][..], b"\x00\x03a/b\x00\x50"].concat())
        );
        for bad in ["", "lumitlab.com/join", cut, named.as_str(), LINK] {
            assert!(bad.parse::<Invite>().is_err(), "{bad}");
        }

        // With its password given, the key changes and the link's own secret
        // is kept beside it, which is what a guest's copy holds.
        let opened = invite.clone().unlocked("correct horse");
        assert!(opened.key != invite.key && opened.link == invite.link);
        assert!(opened.to_string().parse::<Invite>().unwrap() == opened);
        // A name reading the link back would refuse is left out of it, and
        // the rest of the link still reads.
        let odd = Invite {
            addresses: vec![
                "203.0.113.5:40000:47856".into(),
                "192.168.1.20:47856".into(),
            ],
            ..invite.clone()
        };
        let read: Invite = odd.to_string().parse().unwrap();
        assert_eq!(read.addresses, ["192.168.1.20:47856"]);
    }
}
