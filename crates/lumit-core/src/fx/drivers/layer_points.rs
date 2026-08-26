//! Layer points: **another** layer's points stream, brought into this layer's
//! graph (points-stream.md §1.2, §2.3 — the family's cross-layer tap).
//!
//! **In plain terms.** A wire never crosses between layers, and that rule is
//! not being bent here. What crosses is a *reference*: this node names a layer,
//! the way Audio level names the layer whose sound it measures, and hands out
//! whatever points that layer's own producer is making. The canvas draws the
//! named layer as a derived source node and the wire out of it is an ordinary
//! wire on this layer's graph — exactly the shape docs/impl/node-graph.md §1.3
//! settled for Audio level, applied to a stream instead of a number.
//!
//! **It is a source, not a reader.** It declares a Points *output* and no
//! input, so anything that already takes a points wire takes this one without
//! knowing the difference: Clone to points stamps another layer's particles,
//! Connect points webs them up, Points sample counts them.
//!
//! **Which producer?** The first enabled effect on the named layer that makes
//! points at all — asked of the signature, never of a list of names. A layer
//! carrying two producers is a layer whose first one is tapped; a picker row is
//! the obvious upgrade and nobody has asked for one.
//!
//! **A tap reaches one layer, never two.** The stream it hands over is
//! evaluated with the named layer's *own* graph applied, so what a tap reads is
//! what that layer draws — but a tap on the far side reads the documented empty
//! stream rather than hopping again. That is the whole of the recursion
//! argument: two layers naming each other stop at the second hop, calmly, with
//! no visited set and no cycle to detect (K-604).
//!
//! **Everything absent is the empty stream**, which is the labelled no-op a
//! dangling layer reference has always been: no layer named, a layer somebody
//! deleted, a layer with no producer on it, a bypassed producer, a producer
//! whose stream depends on a picture (Scatter and Emit from image, K-599 and
//! K-603), or a second hop. Never a fault.

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The port the tapped stream leaves by — the same `points`/`Points` pair every
/// producer declares (K-472), so a wire cannot tell a tap from a Particulate.
pub const POINTS_PORT: &str = "points";

/// The layer-reference row the tap reads, by parameter id.
pub const SOURCE_PARAM: &str = "source";

/// This driver's catalogue name — the one place the walk names it, since a tap
/// is the one node whose *output* the walk has to fetch rather than substitute.
pub const MATCH_NAME: &str = "layer_points";

const OUTPUTS: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// Layer points' one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "layer_points",
    label = "Layer points",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct LayerPoints {
    /// The layer whose points are tapped — an ordinary layer-reference
    /// parameter (docs/03 §8), with the ordinary degrade-to-nothing on a
    /// dangling id. **Edges never cross layers** (K-471): the canvas draws the
    /// referenced layer as a derived source node and the wire from it renders
    /// this parameter, exactly as Audio level's Audio row works.
    #[layer(label = "Points layer", self_default = false)]
    pub source: bool,
}

/// Layer points' behaviour.
pub struct LayerPointsDef;

impl EffectDef for LayerPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LayerPoints as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    /// The first driver whose output is a **stream** rather than a number
    /// (K-604). No inputs at all: what it reads is named by a parameter, not
    /// fed by a wire, because a wire would have to cross a layer boundary.
    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: OUTPUTS,
        }
    }

    /// **Nothing to push.** A points stream is not a [`Value`], so this node
    /// has no numbers to hand the substitution walk; the stream is fetched by
    /// whoever reads the wire, through the driver walk's own points path
    /// (`fx::driver_stream`). A number socket wired to this port therefore
    /// reads as unwired — the documented no-op, and unreachable through a
    /// validated document, where the type check refused it at commit.
    fn eval_driver(&self, _cx: &DriverCx<'_>, _push: &mut dyn FnMut(&'static str, Value)) {}

    // `driver_window` stays at its default nought: a tap reads the named
    // layer's stream at this frame and nowhere else.
}
