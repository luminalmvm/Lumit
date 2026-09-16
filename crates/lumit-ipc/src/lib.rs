//! `lumit-ipc` - the transport every plugin host talks to its broker over, and
//! the handful of rules they all have to agree on.
//!
//! # In plain terms
//!
//! Lumit hosts other people's compiled code in a second process - a *broker* -
//! and talks to it down a local pipe (docs/12 §2.3). Three hosts do it: the
//! OFX host, the audio-plugin host, and LFX - whose three strings were
//! reserved here before its crate existed (docs/impl/lfx.md §3.1).
//! Each has its own protocol, its own shared-memory ring, and its own idea of
//! what a frame is; what none of them has anything of its own to say about is
//! how a message gets across a pipe, or where the broker executable lives.
//!
//! Those were copied between the first two hosts, and the comment-stripped
//! difference between the copies was one hunk in each: the host's own name,
//! hard-coded inside the function being copied. So this crate is a
//! **parameterisation rather than a move** (docs/impl/lfx.md §3.1). [`pipe_name`]
//! takes the prefix; [`broker_exe`] takes the executable's name and the
//! environment variable that overrides it. Each host keeps those three strings
//! of its own, because they are its identity, and two hosts sharing one would
//! put both brokers' endpoints in one namespace and let either host's
//! environment variable redirect the other's child.
//!
//! Beside the transport sit the rules in [`rules`]: the message cap, the
//! handshake timeout, how many strikes a plugin gets, and the sentence a
//! switched-off one files. They are here because every host must answer them
//! the *same way* - a badge that reads the disabled reason by string equality
//! against one host's constant is a wrong sentence waiting for the second host
//! to arrive - and not merely because sharing saves typing. [`addons`] is the
//! same argument about a place rather than a number: the folder Lumit's own
//! installer writes into is one every host must search and none of them owns.
//!
//! # What deliberately stays out
//!
//! The handshake driver, the protocol enums, the rings and the handle
//! registries. `Ready`, `Challenge` and `Hello` are variants of each host's own
//! message enum, and sharing them would mean a trait per enum to hide the
//! difference - an abstraction for its own sake. The *ordering* rules travel as
//! doc comments instead ([`rules`]), and each host writes its own sixty lines.
//! Pictures, blocks of sound and whatever LFX sends share nothing, so a common
//! ring would be an abstraction with one and a half users
//! (docs/impl/audio-plugins.md §5).
//!
//! # Thread role
//!
//! Plain functions over sockets and child processes. Nothing here keeps state,
//! takes a lock, or touches the GPU.
//!
//! Both [`pipe::send`] and [`pipe::recv`] block on the pipe, and every host
//! keeps them on different threads. The **reader thread** owns the
//! [`pipe::RecvHalf`] and does nothing but [`pipe::recv`]; it holds no lock and
//! takes none. The **caller's thread** owns the [`pipe::SendHalf`] and is the
//! only one that calls [`pipe::send`]. That split is what lets a host wait on a
//! deadline rather than on the plugin, because the thread timing the deadline
//! is never the thread blocked on the answer (docs/14 §1). No lock may be held
//! across either call.

#![forbid(unsafe_code)]

pub mod addons;
pub mod hosts;
pub mod pipe;
pub mod rules;
pub mod spawn;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub use addons::{addons_dir, staging_dir, ADDONS_DIR_NAME, STAGING_DIR_NAME};
pub use hosts::{HostEndpoint, RESERVED};
pub use pipe::{pipe_name, PipeError};
pub use rules::{
    describe_deadline, DISABLED_REASON, HANDSHAKE_TIMEOUT, MAX_MESSAGE_BYTES,
    STRIKES_BEFORE_DISABLED,
};
pub use spawn::{broker_exe, no_console};
