//! The control protocol: every sentence the host and the broker can say.
//!
//! # In plain terms
//!
//! Two programs that have to agree need a fixed, small vocabulary and a version
//! number on the front of it. This module is that vocabulary. The host does the
//! asking ([`HostMessage`]) and the broker does the answering
//! ([`BrokerMessage`]), except for two answers that are really questions — the
//! broker asking for frames it discovered it needs, and the plugin saying
//! something to the user through the message suite.
//!
//! **No pixels travel here.** Every message that involves a picture names a
//! *slot* in the shared-memory ring ([`crate::ipc::shm`]); the bytes are already
//! there. A control message is tens of bytes, a frame is tens of megabytes, and
//! keeping them apart is the whole reason the frame plane exists.
//!
//! **The version is checked before anything is believed.** The broker's first
//! word is [`BrokerMessage::Hello`], and a number that is not
//! [`PROTOCOL_VERSION`] ends the conversation with a sentence the user can read
//! rather than a struct deserialised out of somebody else's layout.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::describe::{Context, PluginDescriptor};
use crate::image::{RectI, RowOrder};
use crate::instance::ParamSnapshot;
use crate::props::PropValue;

/// The version both sides must agree on. Bump it whenever a message changes
/// shape: an old broker beside a new host is a mismatch, not a crash.
pub const PROTOCOL_VERSION: u32 = 1;

/// Which instance a message is about. The host mints these; the broker only
/// ever quotes one back.
pub type InstanceId = u32;

/// Which slot of the ring a picture is in.
pub type Slot = u32;

/// One picture, as the ring slot it is sitting in.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FrameRef {
    /// The clip it belongs to, by the name the plugin gave the clip.
    pub clip: String,
    /// The time it is the frame for.
    pub time: f64,
    /// Where it is.
    pub slot: Slot,
}

/// One picture the broker has discovered it needs and does not have.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FrameWanted {
    /// The clip.
    pub clip: String,
    /// The time.
    pub time: f64,
}

/// How the ring is laid out. Sent once, after the handshake, because the ring
/// is sized once per bundle and never again ([`crate::ipc::shm`]).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RingSpec {
    /// The backing file both processes map.
    pub path: String,
    /// How many slots it holds. Never fewer than three: the note's triple
    /// buffering is the floor.
    pub slots: u32,
    /// How many bytes each slot is, header included.
    pub slot_bytes: u64,
}

/// What the host says.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum HostMessage {
    /// Here is the ring; map it.
    Open {
        /// The ring's layout.
        ring: RingSpec,
    },
    /// Describe every plugin in the bundle.
    Describe,
    /// Make an instance of one of them, with these values in its controls.
    CreateInstance {
        /// The identifier the host will use for it from now on.
        instance: InstanceId,
        /// Which plugin, by its index in [`BrokerMessage::Described`].
        plugin: u32,
        /// Which context.
        context: Context,
        /// Every control's value.
        params: ParamSnapshot,
    },
    /// Replace an instance's values without telling the plugin. This is a
    /// scrub or an undo landing: the plugin reads the new numbers at its next
    /// `paramGetValue` (docs/12 §2.2).
    ParamSnapshot {
        /// Which instance.
        instance: InstanceId,
        /// Every control's value.
        params: ParamSnapshot,
    },
    /// One control changed, and the plugin is to be told, wrapped in
    /// begin/end as the spec requires.
    InstanceChanged {
        /// Which instance.
        instance: InstanceId,
        /// Which control.
        name: String,
        /// Its new value.
        value: PropValue,
        /// `kOfxChangeUserEdited` and friends.
        reason: String,
        /// The time the change was made at.
        time: f64,
    },
    /// Render one frame.
    Render {
        /// Which instance.
        instance: InstanceId,
        /// The frame being asked for.
        time: f64,
        /// The rectangle to render.
        bounds: RectI,
        /// Which way up the pictures are handed to the plugin.
        order: RowOrder,
        /// One picture per input clip, already in the ring.
        inputs: Vec<FrameRef>,
        /// The slot the answer goes in.
        output: Slot,
    },
    /// The answer to [`BrokerMessage::NeedFrames`] — **one shipment**, however
    /// many frames the plugin asked for.
    Frames {
        /// Every frame that was asked for, in the ring.
        frames: Vec<FrameRef>,
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
    /// What the bundle holds. The index into this list is what
    /// [`HostMessage::CreateInstance`] names.
    Described {
        /// One per plugin that described itself successfully.
        plugins: Vec<PluginDescriptor>,
    },
    /// The instance exists.
    Created,
    /// The message was carried out and there is nothing to say about it.
    Done,
    /// The plugin wants frames the host has not sent. Answered with exactly one
    /// [`HostMessage::Frames`], which is the point of asking for the lot at
    /// once (docs/impl/ofx-host.md §4).
    NeedFrames {
        /// Every frame, in one list.
        frames: Vec<FrameWanted>,
    },
    /// The plugin said something through the message suite. The host decides
    /// what to do with it; nothing here is modal.
    Note {
        /// `kOfxMessageError`, `kOfxMessageLog`, and the rest.
        kind: String,
        /// What it said.
        text: String,
    },
    /// The frame is in the ring.
    Rendered {
        /// Where.
        slot: Slot,
        /// What `getFramesNeeded` answered, per clip: first and last frame.
        frames_needed: BTreeMap<String, (f64, f64)>,
        /// The clip the plugin said this frame simply is, if it said so.
        identity_of: Option<String>,
    },
    /// Something went wrong, as a sentence rather than a status code: the host
    /// puts it on a badge and the user reads it.
    Failed {
        /// Which action.
        action: String,
        /// What went wrong.
        message: String,
    },
}
