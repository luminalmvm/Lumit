//! Where a layer's sound goes: the master, or a Bus layer, and the sends
//! that copy it to other buses on the way.
//!
//! A **Bus layer** is a Null layer with `audio_only` set. It has no sound of
//! its own: its rack and its Volume work on what is routed to it. A flag and
//! not a kind of its own, so an older Lumit opens the file and sees a silent
//! Null.

use crate::anim::Property;
use crate::model::{Layer, LayerKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A layer's routing. The default is the master with no sends, which is every
/// layer in a project saved before buses existed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioRoutes {
    /// The bus this layer's own sound goes to instead of the master. A bus that
    /// is gone, or is not a bus, means the master.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<AudioSend>,
}

impl AudioRoutes {
    #[must_use]
    pub fn is_master(&self) -> bool {
        self.output.is_none() && self.sends.is_empty()
    }
}

/// A copy of the layer's sound sent to a bus, after its rack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSend {
    pub bus: Uuid,
    /// How much is sent, in dB like Volume, and keyable like it.
    #[serde(default = "Property::zero")]
    pub level_db: Property,
    /// Sent from before the layer's Volume and Pan rather than after them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pre: bool,
}

impl Layer {
    /// This layer is a Bus layer.
    #[must_use]
    pub fn is_bus(&self) -> bool {
        self.audio_only && matches!(self.kind, LayerKind::Null)
    }
}
