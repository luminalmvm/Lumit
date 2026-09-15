//! Proving who is on the other end of a local pipe.
//!
//! # In plain terms
//!
//! Lumit hosts third-party plugins in separate processes — a broker per bundle
//! — and talks to each one over a local socket or a named pipe (docs/12 §2.3).
//! A local endpoint has a *name*, and a name is something any other program on
//! the machine can also say. That leaves two questions neither the pipe nor the
//! operating system answers on its own:
//!
//! * When the host accepts a connection, is that the broker it started, or
//!   something else that got there first?
//! * When the broker connects, is that Lumit on the other end, or something
//!   else wearing the name?
//!
//! Neither is idle worry. The endpoint names used to be the host's process id
//! and a counter, which anything on the machine could work out; and the
//! programs best placed to do the working out are the *other brokers*, each of
//! which is running somebody else's compiled plugin code. A plugin that can
//! impersonate a broker can feed the host descriptors and pixels of its
//! choosing; one that can impersonate the host can read everything the host
//! would have sent.
//!
//! So this crate does three things:
//!
//! 1. **An unpredictable name.** [`Token`] is 128 bits from the operating
//!    system's random source, which is what the endpoint is named after. A
//!    name nobody can guess is most of the defence, and it costs nothing.
//! 2. **A secret that does not travel in public.** [`Secret`] is 256 bits,
//!    handed to the child down its own standard input rather than on its
//!    command line — `/proc/<pid>/cmdline` is readable by every process on the
//!    machine on Linux, and a command line is in every `ps` listing on all of
//!    them.
//! 3. **A mutual challenge and response.** Each side proves it holds the secret
//!    by answering a nonce the *other* side chose, so neither a recording of an
//!    earlier handshake nor a reflection of this one's own half will do
//!    ([`Proof`]).
//!
//! # The exchange
//!
//! ```text
//!   broker ──► Ready    { nonce: N_b }
//!   host   ──► Challenge{ nonce: N_h, proof: Proof::host(secret, N_b) }
//!                                     └─ the broker checks this, and only then
//!                                        loads the plugin
//!   broker ──► Hello    { version, proof: Proof::broker(secret, N_h) }
//!                                     └─ the host checks this, and only then
//!                                        says anything else
//! ```
//!
//! The two proofs are computed under different tags, so the first cannot be
//! replayed as the second. An impostor connecting to the host does learn
//! `Proof::host(secret, N_b)` for a nonce it chose — but that is a keyed hash
//! under the *host* tag, and what it needs is one under the *broker* tag, which
//! it has no way to produce without the secret.
//!
//! # What this is not
//!
//! It is not a defence against a plugin that has already taken over the
//! application's own process: at that point the secret is simply in memory
//! beside everything else. It raises the floor from "any local process can
//! walk into the conversation" to "you must already be inside it".
//!
//! # Thread role
//!
//! Plain values. No IO except [`Secret::generate`] and [`Token::generate`]
//! reading the operating system's random source, and the two standard-input
//! helpers. Everything here is `Send` and `Sync`.

#![forbid(unsafe_code)]

use std::io::{BufRead, Read, Write};

use serde::{Deserialize, Serialize};

/// Why a peer could not be set up or believed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PeerError {
    /// The operating system would not give us random bytes. There is no
    /// fallback on purpose: a guessable secret is worse than no broker.
    #[error("the system random source is unavailable: {0}")]
    NoRandomness(String),

    /// The secret could not be handed to, or read from, standard input.
    #[error("the broker's credential could not be passed: {0}")]
    Handover(String),

    /// A proof that is not the one this side expected. The peer is not who it
    /// says it is, or does not hold the secret.
    #[error("the peer on the broker pipe could not prove who it is")]
    NotAuthenticated,
}

/// A shorthand.
pub type Result<T> = std::result::Result<T, PeerError>;

/// Random bytes from the operating system.
fn os_random<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0_u8; N];
    getrandom::fill(&mut out).map_err(|e| PeerError::NoRandomness(e.to_string()))?;
    Ok(out)
}

/// Bytes as lower-case hex.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// Hex back to bytes, or `None` for anything that is not exactly `N` bytes of
/// lower- or upper-case hex.
fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N.checked_mul(2)? {
        return None;
    }
    let mut out = [0_u8; N];
    let bytes = text.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let at = i.checked_mul(2)?;
        let pair = bytes.get(at..at.checked_add(2)?)?;
        let text = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// The endpoint name
// ---------------------------------------------------------------------------

/// The unguessable part of one broker's endpoint name.
///
/// 128 bits, which is the same "nobody will meet a collision" figure the roto
/// cache keys on — except that here the property wanted is not just uniqueness
/// but unpredictability, which is why it comes from the operating system rather
/// than from a counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token([u8; 16]);

impl Token {
    /// A fresh token.
    ///
    /// # Errors
    ///
    /// [`PeerError::NoRandomness`].
    pub fn generate() -> Result<Self> {
        Ok(Token(os_random()?))
    }

    /// The token as the hex that goes in a file or pipe name.
    #[must_use]
    pub fn as_name(&self) -> String {
        hex(&self.0)
    }
}

// ---------------------------------------------------------------------------
// The secret
// ---------------------------------------------------------------------------

/// The shared secret one host and one broker hold, and nobody else does.
///
/// Deliberately not `Clone` beyond what the two ends need, not `Debug` in a way
/// that prints it, and not `Display` at all: a secret that turns up in a log
/// line is not a secret. [`Secret::to_handover`] is the one way out, and it
/// exists to be written to a pipe.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret([u8; 32]);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the bytes. `note!`, `tracing` and a panic message all reach
        // for Debug, and any one of them would be enough.
        f.write_str("Secret(<32 bytes>)")
    }
}

impl Secret {
    /// A fresh secret.
    ///
    /// # Errors
    ///
    /// [`PeerError::NoRandomness`].
    pub fn generate() -> Result<Self> {
        Ok(Secret(os_random()?))
    }

    /// The secret as the one line that is written to a child's standard input.
    ///
    /// Hex and a newline, so the child can read exactly one line and know it
    /// has the whole thing without a length prefix or a framing rule.
    #[must_use]
    pub fn to_handover(&self) -> String {
        let mut line = hex(&self.0);
        line.push('\n');
        line
    }

    /// Write the secret to a spawned child's standard input, and close it.
    ///
    /// Closing matters: the child reads one line and would otherwise wait for
    /// an end that never comes if the host died between the two.
    ///
    /// # Errors
    ///
    /// [`PeerError::Handover`].
    pub fn hand_over<W: Write>(&self, mut to: W) -> Result<()> {
        to.write_all(self.to_handover().as_bytes())
            .and_then(|()| to.flush())
            .map_err(|e| PeerError::Handover(e.to_string()))
    }

    /// Read the secret a parent wrote, from this process's standard input.
    ///
    /// Called once, first thing, before any plugin code is loaded — which is
    /// the point: a plugin that could read standard input would be reading the
    /// credential it is supposed to be on the far side of.
    ///
    /// # Errors
    ///
    /// [`PeerError::Handover`].
    pub fn from_stdin() -> Result<Self> {
        Self::read_from(std::io::stdin().lock())
    }

    /// [`Secret::from_stdin`], against any reader, so it can be tested.
    ///
    /// # Errors
    ///
    /// [`PeerError::Handover`].
    pub fn read_from<R: Read>(from: R) -> Result<Self> {
        let mut line = String::new();
        std::io::BufReader::new(from)
            .read_line(&mut line)
            .map_err(|e| PeerError::Handover(e.to_string()))?;
        unhex::<32>(line.trim())
            .map(Secret)
            .ok_or_else(|| PeerError::Handover("the credential line is not 32 bytes of hex".into()))
    }
}

// ---------------------------------------------------------------------------
// The handshake
// ---------------------------------------------------------------------------

/// A number used once, chosen by whichever side is asking.
///
/// Serialisable, because it travels in the protocol's own messages rather than
/// in a channel of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Nonce([u8; 32]);

impl Nonce {
    /// A fresh nonce.
    ///
    /// # Errors
    ///
    /// [`PeerError::NoRandomness`].
    pub fn generate() -> Result<Self> {
        Ok(Nonce(os_random()?))
    }
}

/// One side's answer to the other's nonce.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Proof([u8; 32]);

/// Which side computed a proof. The two tags are what stop an impostor
/// reflecting the half it was just given back as the half it owes.
const HOST_TAG: &[u8] = b"lumit-peer/host/v1";
const BROKER_TAG: &[u8] = b"lumit-peer/broker/v1";

impl Proof {
    fn compute(secret: &Secret, tag: &[u8], nonce: Nonce) -> Self {
        // Keyed, not a plain hash of secret-then-message: `blake3::keyed_hash`
        // is a PRF under the key, which is the property being leaned on.
        let mut input = Vec::with_capacity(tag.len().saturating_add(32));
        input.extend_from_slice(tag);
        input.extend_from_slice(&nonce.0);
        Proof(*blake3::keyed_hash(&secret.0, &input).as_bytes())
    }

    /// The host's proof that it holds the secret, answering the broker's nonce.
    #[must_use]
    pub fn host(secret: &Secret, broker_nonce: Nonce) -> Self {
        Self::compute(secret, HOST_TAG, broker_nonce)
    }

    /// The broker's proof that it holds the secret, answering the host's nonce.
    #[must_use]
    pub fn broker(secret: &Secret, host_nonce: Nonce) -> Self {
        Self::compute(secret, BROKER_TAG, host_nonce)
    }

    /// Whether this is the proof it should be, compared in constant time.
    ///
    /// Constant time because the comparison is against a value an attacker
    /// supplies and can vary: a byte-at-a-time `==` that returns early tells
    /// them how much of their guess was right, and a wrong guess they can
    /// improve is a wrong guess they will improve.
    #[must_use]
    pub fn matches(&self, expected: &Proof) -> bool {
        let mut differences = 0_u8;
        for (a, b) in self.0.iter().zip(expected.0.iter()) {
            differences |= a ^ b;
        }
        differences == 0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn two_secrets_are_not_the_same_secret() {
        let a = Secret::generate().unwrap();
        let b = Secret::generate().unwrap();
        assert_ne!(a.0, b.0, "two generated secrets collided");
        assert_ne!(
            Token::generate().unwrap().as_name(),
            Token::generate().unwrap().as_name()
        );
        assert_eq!(Token::generate().unwrap().as_name().len(), 32);
    }

    #[test]
    fn a_secret_never_prints_itself() {
        let secret = Secret::generate().unwrap();
        let shown = format!("{secret:?}");
        assert_eq!(shown, "Secret(<32 bytes>)");
        assert!(
            !shown.contains(&hex(&secret.0)),
            "Debug leaked the secret: {shown}"
        );
    }

    #[test]
    fn the_secret_survives_the_handover_and_nothing_else_does() {
        let secret = Secret::generate().unwrap();
        let line = secret.to_handover();
        assert_eq!(Secret::read_from(line.as_bytes()).unwrap(), secret);

        // Everything a broken or hostile parent could send instead.
        for rubbish in ["", "\n", "not hex\n", "abcd\n", &"f".repeat(63)] {
            assert!(
                Secret::read_from(rubbish.as_bytes()).is_err(),
                "{rubbish:?} was accepted as a credential"
            );
        }
        // The right length of the wrong alphabet.
        assert!(Secret::read_from("z".repeat(64).as_bytes()).is_err());
    }

    #[test]
    fn each_side_answers_the_other_nonce_and_neither_answer_is_the_other() {
        let secret = Secret::generate().unwrap();
        let n_broker = Nonce::generate().unwrap();
        let n_host = Nonce::generate().unwrap();

        let from_host = Proof::host(&secret, n_broker);
        let from_broker = Proof::broker(&secret, n_host);

        assert!(from_host.matches(&Proof::host(&secret, n_broker)));
        assert!(from_broker.matches(&Proof::broker(&secret, n_host)));

        // The tags are what make this true: without them an impostor could
        // hand the host's own proof straight back as the broker's.
        assert!(!from_host.matches(&Proof::broker(&secret, n_broker)));
        assert!(!Proof::host(&secret, n_host).matches(&from_broker));
    }

    #[test]
    fn a_proof_under_another_secret_is_refused() {
        let ours = Secret::generate().unwrap();
        let theirs = Secret::generate().unwrap();
        let nonce = Nonce::generate().unwrap();
        assert!(!Proof::broker(&theirs, nonce).matches(&Proof::broker(&ours, nonce)));
    }

    #[test]
    fn a_proof_for_another_nonce_is_refused_so_a_recording_is_worthless() {
        let secret = Secret::generate().unwrap();
        let recorded = Nonce::generate().unwrap();
        let fresh = Nonce::generate().unwrap();
        // What an eavesdropper on an earlier handshake would have.
        let old = Proof::broker(&secret, recorded);
        assert!(!old.matches(&Proof::broker(&secret, fresh)));
    }

    #[test]
    fn hex_round_trips_and_rejects_the_wrong_length() {
        let bytes = [0x00, 0x0f, 0xf0, 0xff, 0x5a];
        assert_eq!(hex(&bytes), "000ff0ff5a");
        assert_eq!(unhex::<5>("000ff0ff5a"), Some(bytes));
        assert_eq!(unhex::<5>("000ff0ff5"), None);
        assert_eq!(unhex::<5>("000ff0ff5a00"), None);
        // Upper case is read too: a hand-typed credential in a test should not
        // fail for a reason nobody would guess.
        assert_eq!(unhex::<5>("000FF0FF5A"), Some(bytes));
    }
}
