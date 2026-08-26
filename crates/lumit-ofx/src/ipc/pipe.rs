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
//! allocated for it. A broker that has gone wrong — or something else entirely
//! that has connected to the pipe — must not be able to make the host reserve a
//! gigabyte by claiming a gigabyte is coming. Pictures do not travel here
//! (they are in the ring), so the cap can be small enough to be obviously safe.
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
    GenericFilePath, GenericNamespaced, Listener, ListenerOptions, Stream, ToFsName, ToNsName,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;

/// The largest control message either side will send or accept. Control traffic
/// is descriptors and parameter values; the biggest thing that crosses is a
/// bundle's worth of descriptors, and a Sapphire-sized bundle is still small
/// beside this.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// What can go wrong on the wire.
#[derive(Debug, Error)]
pub enum PipeError {
    /// The pipe itself.
    #[error("the broker pipe failed: {0}")]
    Io(#[from] std::io::Error),
    /// A message that would not encode or would not decode.
    #[error("the broker sent a message this host cannot read: {0}")]
    Encoding(String),
    /// A length prefix bigger than [`MAX_MESSAGE_BYTES`].
    #[error("the broker announced a {0}-byte message, which is past the limit")]
    TooLarge(usize),
    /// The other side went away.
    #[error("the broker closed the pipe")]
    Closed,
}

/// The name of one broker's pipe, in the form the platform wants.
///
/// The identifier is the host's own — a process id and a counter — so two
/// brokers, and two copies of Lumit, never collide.
#[must_use]
pub fn pipe_name(identifier: &str) -> String {
    if cfg!(windows) {
        format!("lumit-ofx-{identifier}.pipe")
    } else {
        let mut path: PathBuf = std::env::temp_dir();
        path.push(format!("lumit-ofx-{identifier}.sock"));
        path.to_string_lossy().into_owned()
    }
}

/// Start listening on a name, before the child is spawned: a child that
/// connects to a name nobody is listening on gets an error, and the race is
/// avoided by never having it.
///
/// # Errors
///
/// [`PipeError::Io`] if the name cannot be claimed.
pub fn listen(name: &str) -> Result<Listener, PipeError> {
    // A Unix socket is a file, and a stale one from a broker that died without
    // tidying up would refuse the bind. Removing it is safe: the name carries
    // this process's own id.
    if !cfg!(windows) {
        let _ = std::fs::remove_file(name);
    }
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

/// The two ends of a connected pipe, so that one thread can read while another
/// writes. Re-exported here rather than named through `interprocess` by every
/// caller: the transport is this module's business, and the broker process
/// should not have to depend on the crate that happens to provide it.
pub use interprocess::local_socket::{RecvHalf, SendHalf};

/// Split a connection into its reading and writing halves.
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
/// [`PipeError`] — encoding, the cap, or the pipe.
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
