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

/// What a link starts with instead when the host set a password.
const PACKED_LOCKED: u8 = 2;

/// How many times a password is hashed over, so that trying one costs
/// something for anyone working through a list of them.
const STRETCH: usize = 1 << 16;

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
    /// The host set a password, which the guest has to give as well as
    /// holding the link. A link that leaks is then not enough to join by.
    pub locked: bool,
}

/// What a password comes to, which is what a host keeps of one. Slow on
/// purpose.
#[must_use]
pub fn lock_of(password: &str) -> [u8; 32] {
    let mut lock = blake3::derive_key("lumit-share 2026 password", password.as_bytes());
    for _ in 0..STRETCH {
        lock = *blake3::hash(&lock).as_bytes();
    }
    lock
}

/// The key the channel is opened with when the host set a password: the
/// link's secret and the password's, neither enough without the other.
#[must_use]
pub fn locked_key(link: &[u8; 32], lock: &[u8; 32]) -> [u8; 32] {
    *blake3::keyed_hash(link, lock).as_bytes()
}

impl Invite {
    /// This invite with its password given, ready to join by. One that
    /// needed none comes back as it was. A wrong password is not found out
    /// here: the host does not answer to it.
    #[must_use]
    pub fn unlocked(mut self, password: &str) -> Invite {
        if self.locked {
            self.key = locked_key(&self.key, &lock_of(password));
            self.locked = false;
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

/// Take `n` bytes off the front of `bytes`.
fn take<'a>(bytes: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (front, rest) = bytes.split_at_checked(n)?;
    *bytes = rest;
    Some(front)
}

fn unpack(mut bytes: &[u8]) -> Option<Invite> {
    let locked = match take(&mut bytes, 1)? {
        [PACKED] => false,
        [PACKED_LOCKED] => true,
        _ => return None,
    };
    let key = take(&mut bytes, 32)?.try_into().ok()?;
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
                let plain = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
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
        locked,
    })
}

impl fmt::Display for Invite {
    /// The link. An address that is not `host:port` is left out of it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut packed = vec![if self.locked { PACKED_LOCKED } else { PACKED }];
        packed.extend_from_slice(&self.key);
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
        Ok(Invite {
            addresses: vec![address.to_owned()],
            relays: Vec::new(),
            key,
            locked: false,
        })
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
            key: std::array::from_fn(|n| n as u8 * 7),
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
    }
}
