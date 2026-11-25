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
/// Two, since a descriptor carries the standard the plugin speaks.
pub const PROTOCOL_VERSION: u32 = 3;

/// Which instance a message is about — the bits of a
/// [`Handle`](crate::ipc::handles::Handle), which the host mints and the broker
/// only ever quotes back.
pub type InstanceId = u32;

pub type Slot = u32;

/// How the ring is laid out. Sent once, after the handshake, because the ring
/// is sized once per module and never again ([`crate::ipc::ring`]).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RingSpec {
    pub path: String,
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
    pub plugin_id: String,
    pub state: Option<Vec<u8>>,
    pub params: Vec<(u32, f64)>,
    /// The rate this bake runs at, in hertz. Carried rather than assumed: a
    /// plugin activated at 48 kHz and handed a 96 kHz export tunes every
    /// filter an octave wrong.
    pub rate: u32,
    pub offline: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum HostMessage {
    /// The host's half of the handshake: a nonce for the broker to answer, and
    /// the host's own answer to the nonce the broker opened with.
    ///
    /// The broker checks the proof **before it opens the module**, so an
    /// impostor at the endpoint never gets the plugin's code to run.
    Challenge {
        nonce: lumit_peer::Nonce,
        proof: lumit_peer::Proof,
    },
    Open {
        ring: RingSpec,
    },
    /// Describe every plugin in the module, except these.
    ///
    /// The list travels **with the question**, so a plugin the user has
    /// switched off is never created and its code never runs. It is
    /// read again at the top of every block batch, on the host's side, where a
    /// switch flicked mid-session can be noticed without asking the broker.
    Describe {
        disabled: Vec<String>,
    },
    CreateInstance {
        instance: InstanceId,
        bring: Bring,
    },
    Process {
        instance: InstanceId,
        input: Slot,
        output: Slot,
        /// The parameter values for this block. Need not be sorted — the
        /// boundary sorts, because CLAP calls an unsorted list undefined.
        events: Vec<ParamEvent>,
        steady: i64,
    },
    Save {
        instance: InstanceId,
    },
    Destroy {
        instance: InstanceId,
    },
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum BrokerMessage {
    /// The actual first word: a nonce, and nothing else. Whoever connected gets
    /// to say this much; it proves nothing and reveals nothing.
    Ready {
        nonce: lumit_peer::Nonce,
    },
    /// The broker's answer to the host's challenge, with the protocol it
    /// speaks. Checked before the version: a peer that cannot prove who it is
    /// has no version worth hearing.
    Hello {
        version: u32,
        proof: lumit_peer::Proof,
    },
    /// The ring is mapped, and the host may now unlink its path. On Unix a
    /// mapping outlives the name it was opened through.
    RingOpened,
    /// The broker could not map the ring the host named. Sent so the host
    /// returns at once rather than waiting out the handshake timeout for an
    /// acknowledgement that is never coming; the ring's file is left where it
    /// is, since the host may yet try again through it.
    RingRefused,
    Described {
        plugins: Vec<PluginDescriptor>,
        rejected: Vec<Refusal>,
    },
    Created {
        /// What it reports now that it is active — the number a describe could
        /// not honestly ask for (docs/impl/audio-plugins.md §4).
        latency: u32,
        warning: Option<String>,
    },
    Processed {
        slot: Slot,
    },
    /// The blob, byte for byte. Never parsed, always round-tripped.
    Saved {
        bytes: Vec<u8>,
    },
    Done,
    /// Something went wrong, as a sentence rather than a status code: the host
    /// puts it on a badge and the user reads it.
    Failed {
        action: String,
        message: String,
    },
}
