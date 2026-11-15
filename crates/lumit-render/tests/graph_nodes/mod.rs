//! Small builders for graph nodes and wires.

use super::*;
use lumit_core::comp_graph::{GraphInput, InputKind};

pub(super) fn wire(from: Uuid, from_port: &str, to: Uuid, to_port: &str) -> GraphEdge {
    GraphEdge {
        from,
        from_port: from_port.to_owned(),
        to,
        to_port: to_port.to_owned(),
    }
}

pub(super) fn read(item: Uuid) -> GraphNode {
    GraphNode::Read {
        id: Uuid::now_v7(),
        item,
        custom_name: None,
    }
}

pub(super) fn picture_input(id: &str) -> GraphNode {
    GraphNode::Input {
        id: Uuid::now_v7(),
        input: GraphInput {
            id: id.into(),
            label: id.into(),
            kind: InputKind::Picture,
            default: [0.0; 4],
            min: 0.0,
            max: 1.0,
            unit: lumit_core::fx::params::Unit::Raw,
            preview: None,
        },
    }
}

pub(super) fn number_input(id: &str, default: f64) -> GraphNode {
    GraphNode::Input {
        id: Uuid::now_v7(),
        input: GraphInput {
            id: id.into(),
            label: id.into(),
            kind: InputKind::Number,
            default: [default, 0.0, 0.0, 0.0],
            min: -8.0,
            max: 8.0,
            unit: lumit_core::fx::params::Unit::Raw,
            preview: None,
        },
    }
}

pub(super) fn output() -> GraphNode {
    GraphNode::Output { id: Uuid::now_v7() }
}
