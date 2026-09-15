//! Split channels (docs/impl/node-graph-comp.md §1.3): the node that hands out
//! a picture's red, green, blue and alpha as four greyscale pictures.
//!
//! **In plain terms.** Wire a picture in and get four out, one per channel, each
//! an opaque grey picture of that channel. Treat one on its own and put them
//! back together with Combine channels.
//!
//! **A node graph's own**, beside the Merge: it has several outputs, which only
//! a graph can wire, so the Compositing category keeps it off a layer stack. The
//! graph walk lowers each output onto a Set channels pass, so there is no kernel
//! of its own.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// Split channels has no controls: the sockets are the whole box.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "split_channels",
    label = "Split channels",
    version = 1,
    category = Compositing,
    // One Set channels pass per output wired.
    cost = Trivial,
    roi = FullFrame,
    // Nothing to dissolve, as the Merge has nothing.
    matte = false,
)]
pub struct SplitChannels;

/// Split channels' behaviour: none of its own, for the Merge's reason.
pub struct SplitChannelsDef;

impl EffectDef for SplitChannelsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SplitChannels as EffectMetadata>::SCHEMA
    }

    /// The graph walk makes its outputs, so the resolve step pushes no op.
    fn is_image_op(&self) -> bool {
        false
    }
}
