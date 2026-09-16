//! The duplex pipe, and length-prefixed frames on it.
//!
//! # In plain terms
//!
//! This module is `lumit_ipc::pipe` wearing this host's name. The pipe, the
//! length prefix, the cap that is checked before a byte is allocated and the
//! reasoning behind all three are there; what stays here is the one thing that
//! is this host's and not the transport's - the endpoint's prefix, which is why
//! [`pipe_name`] takes an identifier and nothing else while the shared one
//! takes both (`ipc::identity`, docs/impl/lfx.md §3.1).
//!
//! Everything else is re-exported rather than re-implemented, so a caller says
//! `ipc::pipe::send` as the older hosts' callers do and no crate outside
//! `lumit-ipc` names the library the endpoint really comes from.

pub use lumit_ipc::pipe::{
    accept, connect, listen, recv, send, split, Listener, PipeError, RecvHalf, SendHalf, Stream,
    MAX_MESSAGE_BYTES,
};

use crate::ipc::identity::HOST_PREFIX;

/// The name of one broker's pipe, in the form the platform wants.
///
/// `identifier` is 128 bits of operating-system randomness in hex
/// ([`lumit_peer::Token`]), never a process id and a counter. A name nobody can
/// guess is most of the defence against the programs best placed to guess it,
/// which are the other brokers - each running somebody else's plugin code.
/// [`lumit_ipc::pipe_name`] has the longer version.
#[must_use]
pub fn pipe_name(identifier: &str) -> String {
    lumit_ipc::pipe_name(HOST_PREFIX, identifier)
}
