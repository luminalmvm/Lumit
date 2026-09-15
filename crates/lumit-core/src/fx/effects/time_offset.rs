//! Time offset (docs/impl/node-graph-comp.md §5.2): the node that shows its
//! input at another time.
//!
//! **In plain terms.** A wire in a node graph means "the picture this box
//! makes", so a box can be asked for that picture at another moment. This is
//! the box that asks: everything wired into it is evaluated a second early, or
//! a second late, and what comes out is that picture. A Read a frame later, an
//! effect chain half a second behind the rest of the graph, one branch of a
//! fork trailing the other - all of them are this box and a number.
//!
//! **A node graph's own**, beside the Merge and the Switch: the Compositing
//! category is not offered on a layer stack, where the same idea is a layer's
//! own start offset. The graph walk realises it by lowering the box's input at
//! the shifted time rather than by running a kernel, which is why it has no CPU
//! oracle and no GPU entry.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The Time offset's one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "time_offset",
    label = "Time offset",
    version = 1,
    category = Compositing,
    // It shows a picture somebody else made, at another moment.
    cost = Trivial,
    roi = FullFrame,
    // Nothing to dissolve, as the Merge has nothing: the box makes no picture
    // of its own.
    matte = false,
)]
pub struct TimeOffset {
    /// How far the input is shifted, in seconds. Positive shows the picture
    /// later in the graph's own clock, negative earlier; keyframeable, so the
    /// shift itself can ramp.
    #[slider(min = -5.0, max = 5.0, default = 0.0, unit = Seconds)]
    pub offset: f32,
}

/// The Time offset's behaviour: none of its own, for the Merge's reason.
pub struct TimeOffsetDef;

impl EffectDef for TimeOffsetDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TimeOffset as EffectMetadata>::SCHEMA
    }

    /// It moves the time its input is read at rather than drawing anything, so
    /// the resolve step pushes no op for it.
    fn is_image_op(&self) -> bool {
        false
    }
}
