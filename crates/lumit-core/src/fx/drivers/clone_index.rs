//! Clone index: which copy a layer is being stamped as.
//!
//! **In plain terms.** Clone to points stamps one layer many times. Put this
//! on the layer being stamped, wire it into anything there, and each copy
//! reads its own number, so every copy can have a different radius, colour or
//! whatever a parameter can do.
//!
//! **It only counts inside Clone to points.** Anywhere else, the layer on its
//! own included, it reads the first copy of one, so the layer still draws.

use crate::comp_graph::GraphNode;
use crate::fx::effects::node_graph;
use crate::fx::{
    points, DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use crate::graph::LayerGraph;
use crate::model::{Document, EffectInstance, Layer, LayerKind};
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// The name the driver is looked up by.
pub const MATCH_NAME: &str = "clone_index";

/// The dice Random rolls, kept off the numbers the points effects use.
const RANDOM_ATTR: u32 = 50;

/// Clone index's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "clone_index",
    label = "Clone index",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct CloneIndex {
    /// Which dice Random rolls.
    #[seed]
    pub seed: u32,
}

/// The port the copy's number leaves by, counted from 0.
pub const INDEX_PORT: &str = "index";
/// The port the number of copies leaves by.
pub const COUNT_PORT: &str = "count";
/// The port the number as a fraction leaves by: 0 at the first copy, 1 at
/// the last.
pub const NORMALISED_PORT: &str = "normalised";
/// The port the copy's own roll leaves by, 0 up to 1.
pub const RANDOM_PORT: &str = "random";

/// Whether `graph` holds a Clone index that is switched on.
#[must_use]
pub fn present(graph: &LayerGraph) -> bool {
    graph
        .nodes
        .iter()
        .any(|n| n.enabled && n.effect.match_name == MATCH_NAME)
}

/// Whether a Clone index that is switched on is anywhere a render of `layer`
/// evaluates drivers: its own graph, the layers of a precomp it shows and the
/// precomps inside that, and a node graph it or one of those layers shows or
/// applies. Clone to points asks this to decide whether a layer needs a
/// render per copy.
///
/// Each composition is looked in once, so comps that show each other stop.
#[must_use]
pub fn reaches(doc: &Document, layer: &Layer) -> bool {
    in_layer(doc, layer, &mut Vec::new())
}

fn in_layer(doc: &Document, layer: &Layer, seen: &mut Vec<Uuid>) -> bool {
    if present(&layer.graph) {
        return true;
    }
    if let LayerKind::Precomp { comp } = &layer.kind {
        if in_comp(doc, *comp, seen) {
            return true;
        }
    }
    applied(doc, &layer.effects, seen)
}

/// The node graphs a stack's Node graph effects apply.
fn applied(doc: &Document, effects: &[EffectInstance], seen: &mut Vec<Uuid>) -> bool {
    effects
        .iter()
        .any(|e| e.enabled && node_graph::comp_of(e).is_some_and(|comp| in_comp(doc, comp, seen)))
}

fn in_comp(doc: &Document, id: Uuid, seen: &mut Vec<Uuid>) -> bool {
    if seen.contains(&id) {
        return false;
    }
    seen.push(id);
    let Some(comp) = doc.comp(id) else {
        return false;
    };
    if comp.layers.iter().any(|l| in_layer(doc, l, seen)) {
        return true;
    }
    // A node graph comp's drivers are boxes of its own, and it may read or
    // apply other comps.
    comp.graph.as_ref().is_some_and(|graph| {
        graph.nodes.iter().any(|node| match node {
            GraphNode::Fx(inst) => {
                inst.enabled
                    && (inst.effect.match_name == MATCH_NAME
                        || applied(doc, std::slice::from_ref(inst), seen))
            }
            GraphNode::Read { item, .. } => in_comp(doc, *item, seen),
            _ => false,
        })
    })
}

/// Clone index's behaviour.
pub struct CloneIndexDef;

impl EffectDef for CloneIndexDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CloneIndex as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        const PORTS: &[Port] = &[
            Port::new(INDEX_PORT, "Index", PortType::Number),
            Port::new(COUNT_PORT, "Count", PortType::Number),
            Port::new(NORMALISED_PORT, "Normalised", PortType::Number),
            Port::new(RANDOM_PORT, "Random", PortType::Number),
        ];
        Signature::Data {
            inputs: &[],
            outputs: PORTS,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let (index, count) = cx.clone;
        let seed = CloneIndex::read(cx.params).seed;
        push(INDEX_PORT, Value::Float(index as f32));
        push(COUNT_PORT, Value::Float(count as f32));
        // One copy has no last to be 1 at, so it reads 0.
        let last = count.saturating_sub(1).max(1);
        push(NORMALISED_PORT, Value::Float(index as f32 / last as f32));
        push(
            RANDOM_PORT,
            Value::Float(points::draw(seed, u64::from(index), RANDOM_ATTR)),
        );
    }
}
