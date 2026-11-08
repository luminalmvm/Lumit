//! A graph box's label colour and its fold, read and written on their own as
//! the notes are: the wiring doesn't carry them. Each write is one undo step,
//! over every box it names.

use flutter_rust_bridge::frb;
use lumit_core::graph::looks::BoxLooks;
use lumit_core::ops::Op;
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::graph::BridgeNodeRef;
use crate::api::layer::LayerReference;
use crate::api::BridgeError;

/// A tinted box on a layer's graph and its chip of the label palette.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeNodeColour {
    pub node: BridgeNodeRef,
    pub colour: u32,
}

/// How a layer graph's boxes are drawn: the tinted ones, and the ones folded
/// down to their heading.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeNodeLooks {
    pub colours: Vec<BridgeNodeColour>,
    pub collapsed: Vec<BridgeNodeRef>,
}

/// A tinted box on a node graph, by its id.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeCompNodeColour {
    pub node: Uuid,
    pub colour: u32,
}

/// The same for a node graph's boxes.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCompNodeLooks {
    pub colours: Vec<BridgeCompNodeColour>,
    pub collapsed: Vec<Uuid>,
}

impl LayerReference {
    /// How this layer's graph draws its boxes.
    #[frb(sync)]
    pub fn get_graph_looks(&self) -> Result<BridgeNodeLooks, BridgeError> {
        let looks = self.item()?.graph.looks;
        Ok(BridgeNodeLooks {
            colours: looks
                .colours
                .into_iter()
                .map(|(node, colour)| BridgeNodeColour {
                    node: BridgeNodeRef::of(node),
                    colour,
                })
                .collect(),
            collapsed: looks.collapsed.into_iter().map(BridgeNodeRef::of).collect(),
        })
    }

    /// Tint `nodes` with chip `colour`; chip 0 takes the tint away.
    #[frb(sync)]
    pub fn set_graph_box_colour(
        &self,
        nodes: Vec<BridgeNodeRef>,
        colour: u32,
    ) -> Result<(), BridgeError> {
        self.restyle(|looks| {
            for node in nodes {
                looks.set_colour(node.core(), colour);
            }
        })
    }

    /// Fold `nodes` down to their heading, or open them again.
    #[frb(sync)]
    pub fn set_graph_box_collapsed(
        &self,
        nodes: Vec<BridgeNodeRef>,
        collapsed: bool,
    ) -> Result<(), BridgeError> {
        self.restyle(|looks| {
            for node in nodes {
                looks.set_collapsed(node.core(), collapsed);
            }
        })
    }

    #[frb(ignore)]
    fn restyle(
        &self,
        change: impl FnOnce(&mut BoxLooks<lumit_core::graph::NodeRef>),
    ) -> Result<(), BridgeError> {
        let mut graph = self.item()?.graph;
        let before = graph.looks.clone();
        change(&mut graph.looks);
        // Nothing to undo when nothing changed.
        if graph.looks == before {
            return Ok(());
        }
        self.commit(Op::SetLayerGraph {
            comp: self.comp_id,
            layer: self.layer_id,
            graph: Box::new(graph),
        })
    }
}

impl CompositionReference {
    /// How this node graph draws its boxes.
    #[frb(sync)]
    pub fn get_node_graph_looks(&self) -> Result<BridgeCompNodeLooks, BridgeError> {
        let doc = self.document()?;
        let looks = &self.graph_of(&doc)?.looks;
        Ok(BridgeCompNodeLooks {
            colours: looks
                .colours
                .iter()
                .map(|&(node, colour)| BridgeCompNodeColour { node, colour })
                .collect(),
            collapsed: looks.collapsed.clone(),
        })
    }

    /// Tint `nodes` with chip `colour`; chip 0 takes the tint away.
    #[frb(sync)]
    pub fn set_node_graph_box_colour(
        &self,
        nodes: Vec<Uuid>,
        colour: u32,
    ) -> Result<(), BridgeError> {
        self.restyle(|looks| {
            for node in nodes {
                looks.set_colour(node, colour);
            }
        })
    }

    /// Fold `nodes` down to their heading, or open them again.
    #[frb(sync)]
    pub fn set_node_graph_box_collapsed(
        &self,
        nodes: Vec<Uuid>,
        collapsed: bool,
    ) -> Result<(), BridgeError> {
        self.restyle(|looks| {
            for node in nodes {
                looks.set_collapsed(node, collapsed);
            }
        })
    }

    #[frb(ignore)]
    fn restyle(&self, change: impl FnOnce(&mut BoxLooks<Uuid>)) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let mut graph = self.graph_of(&doc)?.clone();
        change(&mut graph.looks);
        if graph.looks == self.graph_of(&doc)?.looks {
            return Ok(());
        }
        self.commit(Op::SetCompGraph {
            comp: self.id,
            graph: Box::new(graph),
        })
    }
}
