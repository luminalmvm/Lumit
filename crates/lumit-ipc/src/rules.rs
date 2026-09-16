//! The rules every host must answer the same way.
//!
//! # In plain terms
//!
//! These are not here because sharing saves typing. They are here because a
//! second host answering one of them differently is a bug somewhere else.
//!
//! The clearest case is [`DISABLED_REASON`]. Every hosted effect files its
//! failures into one table, and the seam that puts a badge on the layer decides
//! "switched off" rather than "failed" by comparing the sentence in that table
//! against this constant. While there was one host, the constant could live in
//! it; a second host filing a string of its own would badge its layers *failed* -
//! the same wrong sentence, arriving by a different door
//! (docs/impl/lfx.md §4.3).
//!
//! Which hosts file it today, exactly, because the paragraph above is easy to
//! read as more than is there. The two picture hosts do: `lumit-ofx`'s
//! discovery files this key against a switched-off plugin, and `lumit-lfx`'s
//! `BrokerError::SwitchedOff` has it for a `Display`. The audio host files
//! nothing for that case at all - a switched-off audio plugin is never opened
//! (`a_switched_off_plugin_is_never_described`), the chain heals around the
//! missing link, and the badge is read off the session's own switched-off list
//! rather than off a sentence. It is still this constant's rule that binds it:
//! the table is one table, and the day the audio host has a switched-off layer
//! to file for, this is what it files. What a host's watchdog files after three
//! strikes is a *failure* and reads as one; that sentence is deliberately not
//! this key.
//!
//! ponytail: nothing above fails to compile if a host ignores it. The table
//! being filed into lives two crates away, so a host spelling "switched off"
//! its own way is caught by a badge looking wrong rather than by a test here.
//!
//! The rest are the same argument with smaller consequences: a handshake
//! timeout that differs between hosts is a different program's idea of how long
//! a program takes to start, and a strike count that differs is a different
//! idea of how much patience somebody else's code has earned.
//!
//! # The ordering rules, which travel as words rather than as code
//!
//! The handshake driver itself is **not** shared - `Ready`, `Challenge` and
//! `Hello` are variants of each host's own protocol enum, and a trait per enum
//! to hide that would be an abstraction for its own sake. What must hold in
//! every host is the order, so it is written down here once:
//!
//! 1. The host listens on its endpoint **before** it spawns the child, so a
//!    child that connects to a name nobody is listening on cannot happen.
//! 2. The broker speaks first (`Ready`, carrying its nonce) and loads nothing.
//! 3. The host answers with its own nonce and its proof, and **only then** may
//!    the broker open the stranger's code.
//! 4. The broker answers with its protocol version and its proof, and only then
//!    may the host say anything else. A wrong protocol version is refused
//!    *after* the proof, never before: refusing early tells an impostor which
//!    guess was close.
//! 5. Every host message is answered exactly once, so that a deadline with no
//!    reply is a strike rather than a wait. The exceptions are named per host
//!    and are exactly the messages consumed inside the exchange loop and the
//!    one sent when there is nobody left to answer.

use std::time::Duration;

/// The largest control message either side will send or accept.
///
/// Control traffic is descriptors, parameter values and state blobs; pictures
/// and sound are in each host's ring and never cross here. The biggest thing
/// that does cross is a bundle's worth of descriptors, and a Sapphire-sized
/// bundle is still small beside this.
///
/// **Ceiling:** a plugin whose saved state is bigger than this cannot be hosted
/// through a broker, and says so in a report line rather than crossing. Eight
/// megabytes is a preset library; nothing an effect legitimately remembers
/// about itself comes near it.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// How many consecutive failures a plugin gets before it is put away for the
/// session (docs/12 §2.3).
///
/// A missed deadline and a dead process are the same kind of event: a strike.
/// One or two cost that frame and buy a restart; the third stops trying. A
/// successful action puts the count back to nought - *consecutive* is the word
/// docs/12 §2.3 uses and it is the word every host obeys.
pub const STRIKES_BEFORE_DISABLED: u32 = 3;

/// How long a host waits for a freshly spawned broker to connect and say hello.
///
/// Separate from the action deadlines: this one is about a program starting,
/// not about a plugin thinking.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a switched-off plugin files under its instance.
///
/// Read as a **key** by the seam that badges the layer, never shown verbatim,
/// and read by string equality - which is why it is one constant for every host
/// rather than one each (docs/impl/lfx.md §4.3).
pub const DISABLED_REASON: &str = "plugin_disabled";

/// How long a describe may take: the handshake's ceiling, or a host's own
/// control deadline set longer than it.
///
/// `control_timeout` is whatever the caller's quirks table says for this
/// bundle. The first describe opens the bundle from disk on a process that has
/// only just said hello, which is a program starting rather than a plugin
/// thinking, and nothing on a render waits on it - so the floor is the
/// handshake's, never the shorter of the two.
#[must_use]
pub fn describe_deadline(control_timeout: Duration) -> Duration {
    HANDSHAKE_TIMEOUT.max(control_timeout)
}
