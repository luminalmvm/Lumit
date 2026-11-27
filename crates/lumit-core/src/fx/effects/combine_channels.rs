//! Combine channels (docs/impl/node-graph-comp.md §1.3): the node that builds
//! one picture out of four, Split channels in reverse.
//!
//! **In plain terms.** Each output channel is one channel of the picture wired
//! to that socket, picked on its row, and Luminance by default. A Split hands
//! out greys, whose luminance is the channel, so they go straight back in. An
//! unwired Red, Green or Blue reads nothing and an unwired Alpha reads full on,
//! so three pictures make an opaque one.
//!
//! **A node graph's own**, beside the Merge and for the same reasons. The graph
//! walk lowers it onto three Set channels passes, so there is no kernel of its
//! own.

use crate::fx::{
    builtins::CHANNEL_OPTIONS, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Combine channels' controls: which channel of each socket's picture is read.
/// The ids are not the socket ids, since rows and sockets share names on a box.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "combine_channels",
    label = "Combine channels",
    version = 1,
    category = Compositing,
    cost = Trivial,
    roi = FullFrame,
    matte = false,
)]
pub struct CombineChannels {
    #[choice(label = "Red from", options = *CHANNEL_OPTIONS, default = 0)]
    pub red_from: u32,

    #[choice(label = "Green from", options = *CHANNEL_OPTIONS, default = 0)]
    pub green_from: u32,

    #[choice(label = "Blue from", options = *CHANNEL_OPTIONS, default = 0)]
    pub blue_from: u32,

    #[choice(label = "Alpha from", options = *CHANNEL_OPTIONS, default = 0)]
    pub alpha_from: u32,
}

pub struct CombineChannelsDef;

impl EffectDef for CombineChannelsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CombineChannels as EffectMetadata>::SCHEMA
    }

    /// The graph walk joins its pictures, so the resolve step pushes no op.
    fn is_image_op(&self) -> bool {
        false
    }
}
