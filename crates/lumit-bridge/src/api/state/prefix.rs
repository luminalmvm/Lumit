//! Where the Viewer looks into the picture rather than at the finished one:
//! the "at" chip's point on a layer's stack or a node graph.

use flutter_rust_bridge::frb;
use uuid::Uuid;

/// Where the Viewer cuts a layer's effect stack short — the "at effect" chip's
/// point.
///
/// **In plain terms.** Picking an effect and turning the chip on shows the
/// composition rendered with that layer's stack stopping after the picked
/// effect and nothing past it. The point is named by the layer it is on (which
/// carries its composition) and the effect instance it stops after, which is
/// everything the engine needs to shorten the stack.
///
/// It rides the ordinary render request, so the picture comes back down the one
/// frame transport at the Viewer's own quality — this names a *way of looking*,
/// not a second viewport.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgePrefixPoint {
    /// The layer whose stack is cut, and — through its `comp_id` — the
    /// composition the cut belongs to. A point naming another composition's
    /// layer cuts nothing, so a stale chip is harmless rather than wrong.
    ///
    /// `None` on a node graph's point, which names a comp and a box in
    /// [`Self::graph`] instead: a node graph has no layers to cut.
    pub layer: Option<crate::api::layer::LayerReference>,
    /// The effect instance the stack stops **after**. An effect the layer no
    /// longer carries cuts nothing, for the same reason.
    ///
    /// `None` is the layer's **own picture**, before any effect at all — the
    /// Source box on the node canvas, which is a point on the chain like every
    /// other box and so takes the chip like every other box (N4). The engine
    /// has always been able to answer it (`graph::prefix_len` reads
    /// `NodeRef::Source` as "keep nought"); there was simply no way to say it
    /// here.
    pub effect: Option<Uuid>,
    /// The **node graph's** own point: the box the picture is read at
    /// (docs/impl/node-graph-comp.md §4.5). `None` for the layer's chip and
    /// for no chip at all.
    ///
    /// A field beside the other two rather than a shape of its own, so the
    /// point stays one small copyable record and a caller that names a layer
    /// writes exactly what it wrote before.
    pub graph: Option<BridgeGraphPoint>,
    /// The box inside the Custom shader at this point that the picture is read
    /// at, by its id in the shader's own graph: the shader's Result fed from
    /// that box instead. `None` for the shader's own picture, and on every
    /// point that isn't a Custom shader.
    pub shader_node: Option<u32>,
}

/// Which box of which node graph the Viewer is reading the picture at.
///
/// The comp is named as well as the box because a stale chip must be harmless:
/// a point naming another composition's box cuts nothing, exactly as a layer
/// point naming another composition's layer does.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeGraphPoint {
    pub comp: Uuid,
    pub node: Uuid,
}
