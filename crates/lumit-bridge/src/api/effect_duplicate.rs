//! Duplicate effects in place on their own layer.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::{layer::LayerReference, BridgeError};

impl LayerReference {
    /// Copy each of `effects` onto this layer, each copy right after its
    /// original, as one undo step. Answers the copies' ids, in stack order.
    ///
    /// The same copy a paste makes, keys left where they are: fresh ids, and
    /// any drivers feeding them come along with their wires.
    #[frb(sync)]
    pub fn duplicate_effects(&self, effects: Vec<Uuid>) -> Result<Vec<Uuid>, BridgeError> {
        let layer = self.item()?;
        let taken: Vec<_> = (layer.effects.iter())
            .filter(|e| effects.contains(&e.id))
            .cloned()
            .collect();
        if taken.is_empty() {
            return Err(BridgeError::InvalidEffect);
        }
        let ids: Vec<Uuid> = taken.iter().map(|e| e.id).collect();
        let wiring = lumit_core::preset::wiring_of(&layer.graph, &layer.effects, &ids);
        // Both come out in stack order, so the nth copy is the nth original's.
        let (mut copies, graph) = match &wiring {
            Some(wiring) => {
                let right = (layer.graph.layout.iter())
                    .map(|(_, xy)| xy[0])
                    .fold(f64::MIN, f64::max);
                let at = if right == f64::MIN {
                    0.0
                } else {
                    right + 240.0
                };
                let added = lumit_core::preset::group_instantiated(wiring, [at, 0.0]);
                let mut graph = layer.graph.clone();
                graph.nodes.extend(added.nodes);
                graph.edges.extend(added.edges);
                graph.layout.extend(added.layout);
                (added.effects, Some(graph))
            }
            None => {
                let fresh = taken.into_iter().map(|mut e| {
                    e.id = Uuid::now_v7();
                    e
                });
                (fresh.collect(), None)
            }
        };
        lumit_core::fx::names::name_copies(&layer.effects, &mut copies);
        let made: Vec<Uuid> = copies.iter().map(|e| e.id).collect();
        let mut copies = copies.into_iter();
        let mut stack = Vec::with_capacity(layer.effects.len() + made.len());
        for effect in &layer.effects {
            stack.push(effect.clone());
            if ids.contains(&effect.id) {
                stack.extend(copies.next());
            }
        }
        let Some(mut graph) = graph else {
            self.with_effects(move |effects| {
                *effects = stack;
                Ok(())
            })?;
            return Ok(made);
        };
        graph.prune_rows(&stack);
        self.commit(lumit_core::ops::Op::Batch {
            ops: vec![
                lumit_core::fx::effects::particulate::set_effects(self.comp_id, &layer, stack),
                lumit_core::ops::Op::SetLayerGraph {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    graph: Box::new(graph),
                },
            ],
        })?;
        Ok(made)
    }
}
