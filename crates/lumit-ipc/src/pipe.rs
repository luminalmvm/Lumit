//! The duplex pipe, and length-prefixed frames on it.
//!
//! # In plain terms
//!
//! A pipe is a stream of bytes with no idea where one message ends and the next
//! begins, so every message goes out with its length in front of it: four bytes
//! saying how many follow, then that many bytes of `bincode`. A reader that
//! knows the length can wait for exactly the right amount and never guess.
//!
//! The length is checked against [`MAX_MESSAGE_BYTES`] before a single byte is
//! allocated for it. A broker that has gone wrong - or something else entirely
//! that has connected to the pipe - must not be able to make the host reserve a
//! gigabyte by claiming a gigabyte is coming. Pictures and sound do not travel
//! here (they are in each host's own ring), so the cap can be small enough to
//! be obviously safe.
//!
//! **The name.** On Windows this is a named pipe (`\\.\pipe\…`); everywhere else
//! a Unix socket in the temporary directory. Either way it is a name the host
//! invents per broker and hands to the child on its command line.
//!
//! Why a pipe of its own rather than the child's standard input and output,
//! which would be free: the child loads somebody else's compiled code, and
//! third-party plugins print. One `printf` into standard output would land in
//! the middle of a message and desynchronise the protocol for good.

use std::io::{Read, Write};
use std::path::PathBuf;

use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, ToFsName, ToNsName,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;

pub use crate::rules::MAX_MESSAGE_BYTES;

/// The listening end, the connected end, and the two halves of a connection.
///
/// Re-exported here rather than named through `interprocess` by every caller:
/// the transport is this crate's business, and neither a host nor a broker
/// process should have to depend on the library that happens to provide it.
pub use interprocess::local_socket::{Listener, RecvHalf, SendHalf, Stream};

/// What can go wrong on the wire.
///
/// Worded from **neither end**. A host and its broker call [`send`] and
/// [`recv`] over the same pipe - `lumit-ofx-broker` and its two counterparts
/// read the host's messages with these very functions - so a message that
/// will not decode is "the other end" rather than "the broker": the transport
/// does not know which side of the pipe it is running on, and a line that
/// guessed would name the wrong party in half the logs it appears in.
#[derive(Debug, Error)]
pub enum PipeError {
    /// The pipe itself.
    #[error("the pipe failed: {0}")]
    Io(#[from] std::io::Error),
    /// A message that would not encode or would not decode.
    #[error("the other end sent a message this one cannot read: {0}")]
    Encoding(String),
    /// A length prefix bigger than [`MAX_MESSAGE_BYTES`].
    #[error("the other end announced a {0}-byte message, which is past the limit")]
    TooLarge(usize),
    /// The other side went away.
    #[error("the other end closed the pipe")]
    Closed,
}

/// The name of one broker's pipe, in the form the platform wants.
///
/// `host_prefix` is the calling host's own - `lumit-ofx` and `lumit-aplug`
/// today, `lumit-lfx` when that host arrives - and is an argument rather than a
/// constant here on purpose. Every host spells the rest of the name
/// identically, so lifting the function with one prefix baked in would have put
/// every broker's endpoint in one namespace, where an OFX broker and an LFX
/// broker with the same identifier collide (docs/impl/lfx.md §3.1).
/// [`crate::hosts`] is where the prefixes are reserved against each other.
///
/// `identifier` is 128 bits of operating-system randomness in hex
/// (`lumit_peer::Token`), **not** a process id and a counter as it once was.
/// The old name was unique, which is all a name has to be to keep two brokers
/// apart - but it was also something any other program on the machine could
/// work out, and the programs best placed to work it out are the other brokers,
/// each running somebody else's plugin code. A name nobody can guess is most of
/// the defence and costs nothing.
#[must_use]
pub fn pipe_name(host_prefix: &str, identifier: &str) -> String {
    if cfg!(windows) {
        format!("{host_prefix}-{identifier}.pipe")
    } else {
        let mut path: PathBuf = std::env::temp_dir();
        path.push(format!("{host_prefix}-{identifier}.sock"));
        path.to_string_lossy().into_owned()
    }
}

/// Start listening on a name, before the child is spawned: a child that
/// connects to a name nobody is listening on gets an error, and the race is
/// avoided by never having it.
///
/// The name is **claimed, never cleared**. This used to remove a file at the
/// path first, on the reasoning that a stale socket from a crashed broker would
/// otherwise refuse the bind - true of the old predictable names, and no longer
/// a thing that can happen now the name is random. What that removal did make
/// possible was for a program that had planted something at a predicted path to
/// have it quietly deleted; refusing a name that is already taken is both safer
/// and a better sign that something is wrong.
///
/// # Errors
///
/// [`PipeError::Io`] if the name cannot be claimed, including because something
/// is already there.
pub fn listen(name: &str) -> Result<Listener, PipeError> {
    let options = if cfg!(windows) {
        ListenerOptions::new().name(name.to_ns_name::<GenericNamespaced>()?)
    } else {
        ListenerOptions::new().name(name.to_fs_name::<GenericFilePath>()?)
    };
    Ok(options.create_sync()?)
}

/// Take the one connection a broker makes.
///
/// # Errors
///
/// [`PipeError::Io`].
pub fn accept(listener: &Listener) -> Result<Stream, PipeError> {
    Ok(listener.incoming().next().ok_or(PipeError::Closed)??)
}

/// Split a connection into its reading and writing halves, so that one thread
/// can read while another writes.
#[must_use]
pub fn split(stream: Stream) -> (RecvHalf, SendHalf) {
    stream.split()
}

/// Connect to the host, from inside the broker.
///
/// # Errors
///
/// [`PipeError::Io`] if nobody is listening.
pub fn connect(name: &str) -> Result<Stream, PipeError> {
    let stream = if cfg!(windows) {
        Stream::connect(name.to_ns_name::<GenericNamespaced>()?)?
    } else {
        Stream::connect(name.to_fs_name::<GenericFilePath>()?)?
    };
    Ok(stream)
}

/// Write one message, length first.
///
/// # Errors
///
/// [`PipeError`] - encoding, the cap, or the pipe.
pub fn send<W: Write, M: Serialize>(writer: &mut W, message: &M) -> Result<(), PipeError> {
    let body =
        bincode::serialize(message).map_err(|error| PipeError::Encoding(error.to_string()))?;
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(PipeError::TooLarge(body.len()));
    }
    let length = u32::try_from(body.len()).map_err(|_| PipeError::TooLarge(body.len()))?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Read one message, blocking until it is whole.
///
/// # Errors
///
/// [`PipeError::Closed`] when the other side goes away, and the rest as
/// [`send`].
pub fn recv<R: Read, M: DeserializeOwned>(reader: &mut R) -> Result<M, PipeError> {
    let mut prefix = [0_u8; 4];
    read_exact(reader, &mut prefix)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(PipeError::TooLarge(length));
    }
    let mut body = vec![0_u8; length];
    read_exact(reader, &mut body)?;
    bincode::deserialize(&body).map_err(|error| PipeError::Encoding(error.to_string()))
}

/// `read_exact`, but an empty read is [`PipeError::Closed`] rather than an
/// `UnexpectedEof` the caller would have to unpick.
fn read_exact<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), PipeError> {
    let mut filled = 0;
    while filled < buffer.len() {
        let Some(rest) = buffer.get_mut(filled..) else {
            return Err(PipeError::Closed);
        };
        match reader.read(rest) {
            Ok(0) => return Err(PipeError::Closed),
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(PipeError::Io(error)),
        }
    }
    Ok(())
}
