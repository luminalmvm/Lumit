//! Ties on a graph's wires, read and written as notes are: the whole list at
//! once, one undo step per write.

use flutter_rust_bridge::frb;
use lumit_core::ops::Op;
use lumit_core::wire_tie::WireTie;

use crate::api::composition::CompositionReference;
use crate::api::layer::LayerReference;
use crate::api::BridgeError;

/// A tie gathering some wires at one spot on a graph's canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireTie {
    pub x: f64,
    pub y: f64,
    /// Each wire by the socket it ends in, `box/socket`.
    pub wires: Vec<String>,
}

impl BridgeWireTie {
    fn read(tie: &WireTie) -> Self {
        Self {
            x: tie.at[0],
            y: tie.at[1],
            wires: tie.wires.clone(),
        }
    }

    fn core(self) -> WireTie {
        WireTie {
            at: [self.x, self.y],
            wires: self.wires,
        }
    }
}

impl LayerReference {
    /// The ties on this layer's graph.
    #[frb(sync)]
    pub fn get_graph_ties(&self) -> Result<Vec<BridgeWireTie>, BridgeError> {
        Ok(self
            .item()?
            .graph
            .ties
            .iter()
            .map(BridgeWireTie::read)
            .collect())
    }

    /// Replace the ties on this layer's graph, as one undo step.
    #[frb(sync)]
    pub fn set_graph_ties(&self, ties: Vec<BridgeWireTie>) -> Result<(), BridgeError> {
        let mut graph = self.item()?.graph;
        graph.ties = ties.into_iter().map(BridgeWireTie::core).collect();
        self.commit(Op::SetLayerGraph {
            comp: self.comp_id,
            layer: self.layer_id,
            graph: Box::new(graph),
        })
    }
}

impl CompositionReference {
    /// The ties on this node graph.
    #[frb(sync)]
    pub fn get_node_graph_ties(&self) -> Result<Vec<BridgeWireTie>, BridgeError> {
        let doc = self.document()?;
        Ok(self
            .graph_of(&doc)?
            .ties
            .iter()
            .map(BridgeWireTie::read)
            .collect())
    }

    /// Replace the ties on this node graph, as one undo step.
    #[frb(sync)]
    pub fn set_node_graph_ties(&self, ties: Vec<BridgeWireTie>) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let mut graph = self.graph_of(&doc)?.clone();
        graph.ties = ties.into_iter().map(BridgeWireTie::core).collect();
        self.commit(Op::SetCompGraph {
            comp: self.id,
            graph: Box::new(graph),
        })
    }
}
