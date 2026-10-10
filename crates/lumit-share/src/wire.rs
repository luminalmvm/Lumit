//! What crosses between a host and a guest, and the encrypted channel it
//! crosses in. Used from the reader and writer threads.

use crate::{Person, Presence, Refusal, ShareError};
use lumit_core::{Document, Op};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snow::StatelessTransportState;
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::sync::mpsc::{Receiver as Outbox, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Raised whenever a message changes shape.
pub(crate) const PROTOCOL: u32 = 3;

/// Noise with no long-term keys, both ends proving they hold the invite's
/// secret before anything else is said.
const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";

/// The most one Noise message carries once its tag is taken off.
const CHUNK: usize = 65535 - 16;

/// The longest message that can carry a document. What `lumit-project` reads
/// from a file.
pub(crate) const DOCUMENT_LIMIT: u32 = 512 << 20;

/// The longest message a host reads from a guest, which is one edit.
pub(crate) const EDIT_LIMIT: u32 = 64 << 20;

/// How long either end has to finish the handshake and say hello.
pub(crate) const GREETING: Duration = Duration::from_secs(10);

/// How long a quiet connection is believed. Three missed pings.
pub(crate) const QUIET: Duration = Duration::from_secs(20);

const PING: Duration = Duration::from_secs(5);

/// How often a writer looks for something to send besides its queue.
const TICK: Duration = Duration::from_millis(50);

/// How many messages wait for one connection. A host drops a guest that falls
/// this far behind, and a guest that cannot queue an edit reconnects.
pub(crate) const OUTBOX: usize = 1024;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Message {
    /// A guest's first words.
    Hello {
        protocol: u32,
        version: String,
        schema: String,
        name: String,
        /// A number this guest made up, the same each time it comes back,
        /// which is how the host knows its edits from before. 0 for none.
        #[serde(default)]
        token: u64,
    },
    Refused(Refusal),
    /// The host's answer: who the guest is, the document, and who is here.
    /// `heard` is the last edit the host took from this guest on a connection
    /// before this one, which the document already holds. 0 for none.
    Welcome {
        you: u32,
        heard: u64,
        document: Box<Document>,
        people: Vec<Person>,
    },
    /// An edit a guest made, and what it replaced there, which the host lands
    /// it with. The host answers with `Accepted`, `Applied` or `Rejected`
    /// quoting `id`.
    Submit {
        id: u64,
        op: Op,
        was: Box<Op>,
    },
    /// An edit the host applied, sent in the order it applied them. `peer`
    /// made it and `id` is that peer's own number for it. Its author is sent
    /// it only when it went in differently from how they sent it.
    Applied {
        peer: u32,
        id: u64,
        op: Op,
    },
    /// The host applied this guest's edit as it was sent.
    Accepted {
        id: u64,
    },
    Rejected {
        id: u64,
    },
    People(Vec<Person>),
    Presence {
        peer: u32,
        presence: Presence,
    },
    Ping,
    /// The host has stopped sharing.
    Closed,
    /// The host has taken this guest out of the project.
    Removed,
    /// The host has replaced the invite, having taken someone out. A guest
    /// finds its host by this secret from now on.
    Invite {
        key: [u8; 32],
    },
}

/// File name to this machine's path for it, for every file an effect on this
/// machine names. An effect's file, such as a LUT, crosses as its name only,
/// and each machine puts its own path back.
///
/// Bounded by the distinct files the project's effects name, and dropped when
/// sharing stops.
pub(crate) type Names = Mutex<HashMap<String, String>>;

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\', ':']).next().unwrap_or(path)
}

/// Take everything machine-specific out of a message on its way out, and
/// everything a peer has no business choosing out of one on its way in.
///
/// Done on the JSON, so it holds for every op and every part of the document
/// without a list of them:
/// - no path on the sender's disk crosses. A media reference keeps its file
///   name and fingerprint, which is what the other end finds its copy by.
/// - an effect's file keeps its name, and the receiver puts back its own path
///   for a file of that name if it has one, or else the path of one of that
///   name in `root`, the folder it keeps the project's footage under.
/// - the cache folder is each machine's own. An edit to it becomes an empty
///   batch, which keeps its place in the order and changes nothing.
fn scrub(
    value: &mut Value,
    names: &mut HashMap<String, String>,
    root: Option<&Path>,
    outgoing: bool,
) {
    match value {
        Value::Object(map) => {
            map.remove("absolute_path");
            map.remove("cache_location");
            if let Some(Value::String(path)) = map.get_mut("relative_path") {
                *path = file_name(path).to_owned();
            }
            if map.contains_key("index") {
                if let Some(Value::Array(paths)) = map.get_mut("paths") {
                    for path in paths {
                        let Value::String(path) = path else { continue };
                        let name = file_name(path).to_owned();
                        if outgoing {
                            if name != *path {
                                names.insert(name.clone(), path.clone());
                            }
                            *path = name;
                        } else {
                            // ponytail: the top of the folder only, by name.
                            // A walk, when files kept in folders inside it
                            // have to be found too.
                            let found = names.get(&name).cloned().or_else(|| {
                                let here = root?.join(&name);
                                here.is_file().then(|| here.to_string_lossy().into_owned())
                            });
                            *path = found.unwrap_or(name);
                        }
                    }
                }
            }
            if map.get("op_type").and_then(Value::as_str) == Some("SetCacheLocation") {
                map.clear();
                map.insert("op_type".into(), "Batch".into());
                map.insert("ops".into(), Value::Array(Vec::new()));
            }
            for inner in map.values_mut() {
                scrub(inner, names, root, outgoing);
            }
        }
        Value::Array(items) => {
            for inner in items {
                scrub(inner, names, root, outgoing);
            }
        }
        _ => {}
    }
}

pub(crate) fn encode(message: &Message, names: &Names) -> Result<Arc<[u8]>, ShareError> {
    let mut value = serde_json::to_value(message)?;
    scrub(&mut value, &mut names.lock(), None, true);
    Ok(serde_json::to_vec(&value)?.into())
}

/// Read a message. `root` is the folder this machine keeps the project's
/// footage under, where a file an effect names is looked for.
pub(crate) fn decode(
    bytes: &[u8],
    names: &Names,
    root: Option<&Path>,
) -> Result<Message, ShareError> {
    let mut value: Value = serde_json::from_slice(bytes)?;
    scrub(&mut value, &mut names.lock(), root, false);
    Ok(serde_json::from_value(value)?)
}

/// The sending half of a connection.
pub(crate) struct Sender {
    socket: BufWriter<TcpStream>,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
    sealed: Vec<u8>,
}

/// The receiving half of a connection.
pub(crate) struct Receiver {
    socket: BufReader<TcpStream>,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
    sealed: Vec<u8>,
    plain: Vec<u8>,
}

/// Run the handshake on a fresh connection and split it in two. Both ends
/// have to hold `key`, or it fails here.
pub(crate) fn open(
    mut socket: TcpStream,
    key: &[u8; 32],
    guest: bool,
) -> Result<(Sender, Receiver), ShareError> {
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(GREETING))?;
    socket.set_write_timeout(Some(QUIET))?;
    let builder = snow::Builder::new(PATTERN.parse()?).psk(0, key)?;
    let mut state = if guest {
        builder.build_initiator()?
    } else {
        builder.build_responder()?
    };
    let mut message = [0u8; 256];
    let mut payload = [0u8; 256];
    for turn in [true, false] {
        if turn == guest {
            let n = state.write_message(&[], &mut message)?;
            let len = u16::try_from(n).map_err(|_| ShareError::OutOfTurn)?;
            socket.write_all(&len.to_le_bytes())?;
            socket.write_all(&message[..n])?;
        } else {
            let mut len = [0u8; 2];
            socket.read_exact(&mut len)?;
            let n = usize::from(u16::from_le_bytes(len));
            let theirs = message.get_mut(..n).ok_or(ShareError::OutOfTurn)?;
            socket.read_exact(theirs)?;
            state.read_message(theirs, &mut payload)?;
        }
    }
    let noise = Arc::new(state.into_stateless_transport_mode()?);
    let sender = Sender {
        socket: BufWriter::new(socket.try_clone()?),
        noise: noise.clone(),
        nonce: 0,
        sealed: vec![0; 65535],
    };
    let receiver = Receiver {
        socket: BufReader::new(socket),
        noise,
        nonce: 0,
        sealed: vec![0; 65535],
        plain: vec![0; 65535],
    };
    Ok((sender, receiver))
}

impl Sender {
    /// Send one message: its length, then its bytes in as many Noise messages
    /// as they need.
    pub(crate) fn send(&mut self, bytes: &[u8]) -> Result<(), ShareError> {
        let len = u32::try_from(bytes.len()).map_err(|_| ShareError::TooLarge {
            size: bytes.len() as u64,
            limit: u64::from(u32::MAX),
        })?;
        self.chunk(&len.to_le_bytes())?;
        for chunk in bytes.chunks(CHUNK) {
            self.chunk(chunk)?;
        }
        self.socket.flush()?;
        Ok(())
    }

    fn chunk(&mut self, plain: &[u8]) -> Result<(), ShareError> {
        let n = self
            .noise
            .write_message(self.nonce, plain, &mut self.sealed)?;
        self.nonce += 1;
        let len = u16::try_from(n).map_err(|_| ShareError::OutOfTurn)?;
        self.socket.write_all(&len.to_le_bytes())?;
        self.socket.write_all(&self.sealed[..n])?;
        Ok(())
    }

    /// Close the connection for both halves.
    pub(crate) fn close(&mut self) {
        let _ = self.socket.flush();
        let _ = self.socket.get_ref().shutdown(Shutdown::Both);
    }
}

impl Receiver {
    /// How long to wait for the other end before calling it gone.
    pub(crate) fn patience(&self, wait: Duration) {
        let _ = self.socket.get_ref().set_read_timeout(Some(wait));
    }

    /// Receive one message of at most `limit` bytes. The buffer grows as the
    /// bytes arrive, so a length that lies reserves nothing.
    pub(crate) fn recv(&mut self, limit: u32) -> Result<Vec<u8>, ShareError> {
        let head = self.chunk()?;
        let len = match self.plain.get(..head) {
            Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
            _ => return Err(ShareError::OutOfTurn),
        };
        if len > limit {
            return Err(ShareError::TooLarge {
                size: u64::from(len),
                limit: u64::from(limit),
            });
        }
        let len = len as usize;
        let mut bytes = Vec::new();
        while bytes.len() < len {
            let n = self.chunk()?;
            if n == 0 || bytes.len() + n > len {
                return Err(ShareError::OutOfTurn);
            }
            bytes.extend_from_slice(&self.plain[..n]);
        }
        Ok(bytes)
    }

    /// Read and open one Noise message into `self.plain`.
    fn chunk(&mut self) -> Result<usize, ShareError> {
        let mut len = [0u8; 2];
        self.socket.read_exact(&mut len)?;
        let sealed = &mut self.sealed[..usize::from(u16::from_le_bytes(len))];
        self.socket.read_exact(sealed)?;
        let n = self
            .noise
            .read_message(self.nonce, sealed, &mut self.plain)?;
        self.nonce += 1;
        Ok(n)
    }
}

/// What waits in a connection's queue.
pub(crate) enum Out {
    Bytes(Arc<[u8]>),
    /// Send what is queued ahead of this, then close.
    Close,
}

/// A connection's writer: send what is queued until the queue is dropped or
/// says to close, with a ping when there has been nothing to say. `extra` is
/// asked every tick for something to send that is not worth queueing, which
/// is how a guest's presence goes out latest-wins.
pub(crate) fn write_loop(
    mut sender: Sender,
    outbox: &Outbox<Out>,
    names: &Names,
    mut extra: impl FnMut() -> Option<Message>,
) {
    let mut said = Instant::now();
    loop {
        let bytes = match outbox.recv_timeout(TICK) {
            Ok(Out::Bytes(bytes)) => bytes,
            Ok(Out::Close) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                let message = match extra() {
                    Some(message) => message,
                    None if said.elapsed() >= PING => Message::Ping,
                    None => continue,
                };
                let Ok(bytes) = encode(&message, names) else {
                    continue;
                };
                bytes
            }
        };
        if sender.send(&bytes).is_err() {
            break;
        }
        said = Instant::now();
    }
    sender.close();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A peer is only ever trusted with file names. Paths typed straight into
    /// the JSON, as a hostile one would send them, never reach this machine's
    /// disk: a media reference is cut down to its file name, a path this
    /// machine is meant to have found for itself is thrown away, and the cache
    /// folder cannot be moved at all.
    #[test]
    fn a_peer_cannot_name_a_path_on_this_machine() {
        let nothing = serde_json::json!({ "op_type": "Batch", "ops": [] });
        let hostile = serde_json::json!({ "Submit": { "id": 1, "was": nothing, "op": {
            "op_type": "Batch",
            "ops": [
                {
                    "op_type": "SetMediaRef",
                    "id": "00000000-0000-0000-0000-000000000001",
                    "media": {
                        "relative_path": r"..\..\somewhere\else/clip.mp4",
                        "absolute_path": r"\\elsewhere\share\clip.mp4",
                    },
                },
                {
                    "op_type": "SetCacheLocation",
                    "location": { "Custom": { "folder": r"\\elsewhere\share" } },
                },
            ],
        }}});
        let bytes = serde_json::to_vec(&hostile).unwrap();
        let message = decode(&bytes, &Names::default(), None).unwrap();
        let Message::Submit {
            op: Op::Batch { ops },
            ..
        } = message
        else {
            panic!("a submitted batch");
        };
        let Op::SetMediaRef { media, .. } = &ops[0] else {
            panic!("the relink");
        };
        assert_eq!(media.relative_path, "clip.mp4");
        assert_eq!(media.absolute_path, "");
        assert_eq!(ops[1], Op::Batch { ops: Vec::new() });
    }
}
