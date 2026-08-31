//! The control protocol: every sentence the host and the broker can say.
//!
//! # In plain terms
//!
//! Two programs that have to agree need a fixed, small vocabulary and a version
//! number on the front of it. This module is that vocabulary. The host does the
//! asking ([`HostMessage`]) and the broker does the answering
//! ([`BrokerMessage`]).
//!
//! **No sound travels here.** Every message that involves a block of audio
//! names a *slot* in the shared-memory ring ([`crate::ipc::ring`]); the samples
//! are already there.
//!
//! **The version is checked before anything is believed.** The broker's first
//! word is [`BrokerMessage::Hello`], and a number that is not
//! [`PROTOCOL_VERSION`] ends the conversation with a sentence the user can read
//! rather than a struct deserialised out of somebody else's layout.

use serde::{Deserialize, Serialize};

use crate::describe::{PluginDescriptor, Refusal};
use crate::process::ParamEvent;

/// The version both sides must agree on. Bump it whenever a message changes
/// shape: an old broker beside a new host is a mismatch, not a crash.
///
/// Two, since a descriptor carries the standard the plugin speaks (K-707).
pub const PROTOCOL_VERSION: u32 = 2;

/// Which instance a message is about — the bits of a
/// [`Handle`](crate::ipc::handles::Handle), which the host mints and the broker
/// only ever quotes back.
pub type InstanceId = u32;

/// Which slot of the ring a block of sound is in.
pub type Slot = u32;

/// How the ring is laid out. Sent once, after the handshake, because the ring
/// is sized once per module and never again ([`crate::ipc::ring`]).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RingSpec {
    /// The backing file both processes map.
    pub path: String,
    /// How many slots it holds.
    pub slots: u32,
    /// How many bytes each slot is, header included.
    pub slot_bytes: u64,
}

/// What one live plugin is brought up with, as it crosses the pipe.
///
/// The same three things [`InstanceSetup`](crate::def::InstanceSetup) carries,
/// spelled again here so the protocol does not depend on a type the catalogue
/// side may grow fields on for its own reasons.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Bring {
    /// Which plugin inside the module, by its own stable id.
    pub plugin_id: String,
    /// The blob the `.lum` saved, if this instance has been here before.
    pub state: Option<Vec<u8>>,
    /// The values the project holds, by the plugin's own parameter id.
    pub params: Vec<(u32, f64)>,
    /// Whether this is an export.
    pub offline: bool,
}

/// What the host says.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum HostMessage {
    /// Here is the ring; map it.
    Open {
        /// The ring's layout.
        ring: RingSpec,
    },
    /// Describe every plugin in the module, except these.
    ///
    /// The list travels **with the question**, so a plugin the user has
    /// switched off is never created and its code never runs (K-594). It is
    /// read again at the top of every block batch, on the host's side, where a
    /// switch flicked mid-session can be noticed without asking the broker.
    Describe {
        /// The plugin identifiers to leave alone.
        disabled: Vec<String>,
    },
    /// Make an instance of one of them.
    CreateInstance {
        /// The handle the host will use for it from now on.
        instance: InstanceId,
        /// Which plugin, and what to bring it up with.
        bring: Bring,
    },
    /// One block of sound: the input is in `input`, the answer goes in
    /// `output`.
    Process {
        /// Which instance.
        instance: InstanceId,
        /// Where the input block is.
        input: Slot,
        /// The slot the answer goes in.
        output: Slot,
        /// The parameter values for this block. Need not be sorted — the
        /// boundary sorts, because CLAP calls an unsorted list undefined.
        events: Vec<ParamEvent>,
        /// The running frame count since the chain started.
        steady: i64,
    },
    /// Hand back the blob to write into the `.lum`.
    Save {
        /// Which instance.
        instance: InstanceId,
    },
    /// Destroy an instance.
    Destroy {
        /// Which one.
        instance: InstanceId,
    },
    /// Unload and exit.
    Shutdown,
}

/// What the broker says.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum BrokerMessage {
    /// The first word, before anything else is believed.
    Hello {
        /// The protocol the broker speaks.
        version: u32,
    },
    /// What the module holds.
    Described {
        /// One per plugin that described itself successfully.
        plugins: Vec<PluginDescriptor>,
        /// One calm line per plugin turned away, and why.
        rejected: Vec<Refusal>,
    },
    /// The instance exists.
    Created {
        /// What it reports now that it is active — the number a describe could
        /// not honestly ask for (docs/impl/audio-plugins.md §4).
        latency: u32,
        /// What went wrong bringing it up that did not stop it coming up: a
        /// refused state blob, most often.
        warning: Option<String>,
    },
    /// The block is in the ring.
    Processed {
        /// Where.
        slot: Slot,
    },
    /// The blob, byte for byte. Never parsed, always round-tripped.
    Saved {
        /// The plugin's own memory of itself.
        bytes: Vec<u8>,
    },
    /// The message was carried out and there is nothing to say about it.
    Done,
    /// Something went wrong, as a sentence rather than a status code: the host
    /// puts it on a badge and the user reads it.
    Failed {
        /// Which action.
        action: String,
        /// What went wrong.
        message: String,
    },
}
