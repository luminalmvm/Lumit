//! The evaluation graph: a composition lowered into an immutable DAG of typed
//! nodes (docs/06-RENDER-PIPELINE.md §1.1–§1.2). This module is the compiler
//! and its metadata pass — the graph *structure* and the pass-through folding;
//! the pixel pass that evaluates the graph on the GPU comes in a later slice.
//!
//! In plain terms: before rendering, Kiriko turns a composition into a wiring
//! diagram — "decode this footage, retime it, mask it, place it, then blend it
//! over everything beneath it". A layer that does nothing at a stage (no masks,
//! no retime) simply has no node for that stage, so the renderer never spends
//! a moment on a no-op. Users never see this graph; it is rebuilt whenever the
//! document changes and every in-flight render keeps the graph it started with.

use kiriko_core::model::{BlendMode, Composition, LayerKind};
use uuid::Uuid;

/// An index into an [`EvalGraph`]'s node list. Stable only within one graph.
pub type NodeId = usize;

/// What a [`NodeKind::Source`] fetches or rasterises (docs/06 §1.2 step 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRef {
    Footage(Uuid),
    Solid(Uuid),
    Precomp(Uuid),
    Text,
    Sequence,
}

/// One typed node in the evaluation graph (docs/06 §1.1). The pixel pass gives
/// each kind its rendering behaviour; here they carry only the identity a
/// render and its cache key depend on.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// Fetch or rasterise a layer's source at the resolved source time.
    Source { layer: Uuid, source: SourceRef },
    /// Frame-interpolation retime of a Footage source (present only when the
    /// layer carries a retime; otherwise folded away).
    Retime,
    /// The layer's mask stack gating its alpha (present only when non-empty).
    Masks { count: usize },
    /// Anchor, position, scale, rotation and opacity as one 4×4 transform.
    Transform { layer: Uuid },
    /// Blend the layer's output over the accumulated composite beneath it,
    /// applying its blend mode, track matte and opacity.
    Composite {
        layer: Uuid,
        blend: BlendMode,
        has_matte: bool,
    },
    /// The composition's final pixels: the background with every layer
    /// composited onto it.
    CompOutput { comp: Uuid, width: u32, height: u32 },
}

/// A node and the ids of the nodes feeding it. A [`NodeKind::Composite`] takes
/// `[layer_top]` for the bottom-most layer, or `[layer_top, accumulator]` once
/// there is a composite beneath it.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub inputs: Vec<NodeId>,
}

/// The immutable evaluation graph for one composition. Node ids are indices;
/// `output` is the [`NodeKind::CompOutput`] the pixel pass pulls from.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalGraph {
    pub nodes: Vec<Node>,
    pub output: NodeId,
}

impl EvalGraph {
    fn push(&mut self, kind: NodeKind, inputs: Vec<NodeId>) -> NodeId {
        self.nodes.push(Node { kind, inputs });
        self.nodes.len() - 1
    }

    /// The node at `id`.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// Number of nodes in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the graph has no nodes (never happens after [`compile`], which
    /// always emits at least a [`NodeKind::CompOutput`]).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Iterate every node's kind — handy for tests and metadata queries.
    pub fn kinds(&self) -> impl Iterator<Item = &NodeKind> {
        self.nodes.iter().map(|n| &n.kind)
    }
}

/// Lower a composition into its evaluation graph (docs/06 §1.1). Layers
/// composite bottom-first (index 0 is the top of the stack); each visual layer
/// becomes the ordered chain source → [retime] → [masks] → transform, and a
/// composite node blends it over the accumulated result below. Pass-through
/// stages are folded away: a Footage layer with no retime gets no retime node,
/// a layer with no masks gets no masks node (docs/06 §1.1 identity folding).
/// Camera layers are viewpoints, not pixels, so they are never composited.
pub fn compile(comp: &Composition) -> EvalGraph {
    let mut g = EvalGraph {
        nodes: Vec::new(),
        output: 0,
    };
    let mut acc: Option<NodeId> = None;
    for layer in comp.layers.iter().rev() {
        let Some(source) = source_ref(&layer.kind) else {
            continue; // a camera contributes no pixels
        };
        let mut top = g.push(
            NodeKind::Source {
                layer: layer.id,
                source,
            },
            vec![],
        );
        // Retime folds away unless this is a Footage layer carrying one.
        if matches!(
            layer.kind,
            LayerKind::Footage {
                retime: Some(_),
                ..
            }
        ) {
            top = g.push(NodeKind::Retime, vec![top]);
        }
        // Masks fold away when the layer has none.
        if !layer.masks.is_empty() {
            top = g.push(
                NodeKind::Masks {
                    count: layer.masks.len(),
                },
                vec![top],
            );
        }
        top = g.push(NodeKind::Transform { layer: layer.id }, vec![top]);
        let inputs = match acc {
            Some(below) => vec![top, below],
            None => vec![top],
        };
        acc = Some(g.push(
            NodeKind::Composite {
                layer: layer.id,
                blend: layer.blend,
                has_matte: layer.matte.is_some(),
            },
            inputs,
        ));
    }
    let output = g.push(
        NodeKind::CompOutput {
            comp: comp.id,
            width: comp.width,
            height: comp.height,
        },
        acc.into_iter().collect(),
    );
    g.output = output;
    g
}

/// The source a layer kind fetches, or None for a camera (no pixels).
fn source_ref(kind: &LayerKind) -> Option<SourceRef> {
    Some(match kind {
        LayerKind::Footage { item, .. } => SourceRef::Footage(*item),
        LayerKind::Solid { def } => SourceRef::Solid(*def),
        LayerKind::Precomp { comp } => SourceRef::Precomp(*comp),
        LayerKind::Text { .. } => SourceRef::Text,
        LayerKind::Sequence { .. } => SourceRef::Sequence,
        LayerKind::Camera { .. } => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use kiriko_core::anim::Property;
    use kiriko_core::mask::Mask;
    use kiriko_core::model::{Composition, LayerKind, LinearColour, Switches, TransformGroup};
    use kiriko_core::retime::Retime;
    use kiriko_core::time::{CompTime, Duration, FrameRate, Rational};

    fn secs(s: i64) -> CompTime {
        CompTime(Rational::new(s, 1).unwrap())
    }

    fn layer(kind: LayerKind, masks: Vec<Mask>) -> kiriko_core::model::Layer {
        kiriko_core::model::Layer {
            id: Uuid::now_v7(),
            name: "l".into(),
            kind,
            in_point: secs(0),
            out_point: secs(5),
            start_offset: secs(0),
            transform: TransformGroup::default(),
            matte: None,
            blend: BlendMode::Normal,
            masks,
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn footage(retime: Option<Retime>, masks: Vec<Mask>) -> kiriko_core::model::Layer {
        layer(
            LayerKind::Footage {
                item: Uuid::now_v7(),
                retime,
            },
            masks,
        )
    }

    fn comp_with(layers: Vec<kiriko_core::model::Layer>) -> Composition {
        Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers,
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    fn ident_retime() -> Retime {
        Retime::identity(Rational::new(5, 1).unwrap(), Rational::ZERO)
    }

    #[test]
    fn a_plain_footage_layer_folds_retime_and_masks_away() {
        let g = compile(&comp_with(vec![footage(None, Vec::new())]));
        let kinds: Vec<_> = g.kinds().collect();
        // Source → Transform → Composite → CompOutput, nothing folded in.
        assert_eq!(kinds.len(), 4);
        assert!(matches!(kinds[0], NodeKind::Source { .. }));
        assert!(matches!(kinds[1], NodeKind::Transform { .. }));
        assert!(matches!(kinds[2], NodeKind::Composite { .. }));
        assert!(matches!(g.node(g.output).kind, NodeKind::CompOutput { .. }));
        // No retime and no masks nodes for a plain layer.
        assert!(!g.kinds().any(|k| matches!(k, NodeKind::Retime)));
        assert!(!g.kinds().any(|k| matches!(k, NodeKind::Masks { .. })));
    }

    #[test]
    fn retime_and_masks_appear_only_when_present() {
        let g = compile(&comp_with(vec![footage(
            Some(ident_retime()),
            vec![Mask::rectangle(0.0, 0.0, 10.0, 10.0)],
        )]));
        // Source → Retime → Masks → Transform → Composite → CompOutput.
        let kinds: Vec<_> = g.kinds().collect();
        assert_eq!(kinds.len(), 6);
        assert!(matches!(kinds[0], NodeKind::Source { .. }));
        assert!(matches!(kinds[1], NodeKind::Retime));
        assert!(matches!(kinds[2], NodeKind::Masks { count: 1 }));
        assert!(matches!(kinds[3], NodeKind::Transform { .. }));
        assert!(matches!(kinds[4], NodeKind::Composite { .. }));
        assert!(matches!(kinds[5], NodeKind::CompOutput { .. }));
    }

    #[test]
    fn layers_composite_bottom_first_and_chain_through_the_accumulator() {
        // Two footage layers; index 0 is the top of the stack.
        let (top, bottom) = (footage(None, Vec::new()), footage(None, Vec::new()));
        let (top_id, bottom_id) = (top.id, bottom.id);
        let g = compile(&comp_with(vec![top, bottom]));
        // The bottom layer composites first (single input), the top composites
        // over it (two inputs), and CompOutput pulls from the top composite.
        let composites: Vec<_> = g
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Composite { layer, .. } => Some((*layer, n.inputs.len())),
                _ => None,
            })
            .collect();
        assert_eq!(composites, vec![(bottom_id, 1), (top_id, 2)]);
        let out = g.node(g.output);
        assert_eq!(out.inputs.len(), 1);
        // The output's input is the last (top) composite.
        assert!(matches!(
            g.node(out.inputs[0]).kind,
            NodeKind::Composite { layer, .. } if layer == top_id
        ));
    }

    #[test]
    fn a_camera_layer_contributes_no_pixels() {
        let cam = layer(
            LayerKind::Camera {
                zoom: Property::fixed(1000.0),
            },
            Vec::new(),
        );
        let g = compile(&comp_with(vec![cam, footage(None, Vec::new())]));
        // Only the footage layer composites; the camera has no source node.
        assert_eq!(
            g.kinds()
                .filter(|k| matches!(k, NodeKind::Source { .. }))
                .count(),
            1
        );
        assert_eq!(
            g.kinds()
                .filter(|k| matches!(k, NodeKind::Composite { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn an_empty_comp_is_just_a_comp_output() {
        let g = compile(&comp_with(Vec::new()));
        assert_eq!(g.len(), 1);
        assert!(matches!(g.node(g.output).kind, NodeKind::CompOutput { .. }));
        assert!(g.node(g.output).inputs.is_empty());
    }
}
