//! Switch (docs/impl/node-graph-comp.md §1.3): the node that shows one of its
//! pictures.
//!
//! **In plain terms.** Wire two or three versions of a shot into one box and
//! pick which one comes out with a number. It is how a graph carries an
//! alternative without a second graph, and because the number is an ordinary
//! parameter it keyframes and takes a driver wire - a Wiggle into Index cuts
//! between the pictures on its own.
//!
//! Not a layer's switches, which stay the per-layer toggles they were.
//!
//! **A node graph's own**, beside the Merge and for the same reasons: the
//! Compositing category is not offered on a layer stack, and the graph walk
//! realises the choice by handing on the chosen picture rather than by running
//! a kernel - so there is no CPU oracle and no GPU entry.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The Switch's one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "switch",
    label = "Switch",
    version = 1,
    category = Compositing,
    // It hands on a picture somebody else made.
    cost = Trivial,
    roi = FullFrame,
    // Nothing to dissolve, as the Merge has nothing: the box makes no picture
    // of its own.
    matte = false,
)]
pub struct Switch {
    /// Which socket comes out, counting from zero. Open above, because the box
    /// grows a spare socket every time the last one is wired; out of range
    /// reads transparent, which is what an unwired socket reads anyway.
    #[counter(min = 0, max = 8, default = 0, hard_min = 0, unit = Raw)]
    pub index: i32,
}

/// The Switch's behaviour: none of its own, for the Merge's reason.
pub struct SwitchDef;

impl EffectDef for SwitchDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Switch as EffectMetadata>::SCHEMA
    }

    /// It picks between pictures the graph walk hands it rather than drawing
    /// one, so the resolve step pushes no op for it.
    fn is_image_op(&self) -> bool {
        false
    }
}
