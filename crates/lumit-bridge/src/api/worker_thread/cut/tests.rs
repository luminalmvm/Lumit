#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::super::still_quality;
use super::super::tests::{project_with_solid_of, worker_state};
use uuid::Uuid;

/// **The Viewer's own prefix cut, on the interactive path**.
///
/// Three things, and each is a way the chip could look as if it worked
/// while showing the wrong picture:
///
/// - the cut really shortens the stack, and only the named layer's;
/// - the cut frame **names its own frame** — the key hashes each layer's
///   effects, so a shorter stack is a different name and the cache can
///   never hand the chip the Viewer's full picture (WP5 proved this for
///   the thumbnail; the interactive path is where it now matters);
/// - a point that names nothing this document has cuts nothing, so a chip
///   left over from a deleted effect shows the honest picture rather than
///   an empty one.
#[test]
fn a_prefix_point_cuts_the_stack_and_names_its_own_frame() {
    use crate::api::state::prefix::BridgePrefixPoint;

    let (project, comp) = project_with_solid_of(4);
    let layer_id = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot().comp(comp).expect("comp").layers[0].id
    };
    let (first, second) = (
        lumit_core::fx::builtins::instantiate("exposure").expect("a built-in"),
        lumit_core::fx::builtins::instantiate("exposure").expect("a built-in"),
    );
    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(lumit_core::ops::Op::SetLayerEffects {
                comp,
                layer: layer_id,
                effects: vec![first.clone(), second.clone()],
            })
            .expect("two effects");
    }
    let document = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot()
    };
    let stack = |doc: &std::sync::Arc<lumit_core::model::Document>| {
        doc.comp(comp)
            .expect("comp")
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .expect("the layer")
            .effects
            .len()
    };
    assert_eq!(stack(&document), 2);

    let project_id = project.id;
    let at = |effect| BridgePrefixPoint {
        layer: Some(crate::api::layer::LayerReference::new(
            project_id, comp, layer_id,
        )),
        effect: Some(effect),
        graph: None,
        shader_node: None,
    };
    // The Source box: the layer's own picture, before any effect (N4).
    let at_source = BridgePrefixPoint {
        layer: Some(crate::api::layer::LayerReference::new(
            project_id, comp, layer_id,
        )),
        effect: None,
        graph: None,
        shader_node: None,
    };
    assert_eq!(
        stack(&super::viewed(Some(at_source), document.clone())),
        0,
        "at the Source nothing of the stack is kept"
    );
    assert_eq!(
        stack(&super::viewed(Some(at(first.id)), document.clone())),
        1,
        "at the first effect the stack stops after it"
    );
    assert_eq!(
        stack(&super::viewed(Some(at(second.id)), document.clone())),
        2,
        "at the last effect nothing is cut, so the Viewer's own frame serves"
    );
    assert_eq!(
        stack(&super::viewed(Some(at(Uuid::now_v7())), document.clone())),
        2,
        "an effect the layer no longer carries cuts nothing"
    );
    assert_eq!(
        stack(&super::viewed(None, document.clone())),
        2,
        "and the chip off is the document as it stands"
    );

    // The name, which is the half a wrong cache would get away with.
    let Some(mut state) = worker_state(project) else {
        return;
    };
    let quality = still_quality(1.0);
    let cut = super::viewed(Some(at(first.id)), document.clone());
    let full = state
        .renderer
        .frame_key(&document, comp, 0, quality)
        .expect("a solid is nameable");
    let short = state
        .renderer
        .frame_key(&cut, comp, 0, quality)
        .expect("and so is the cut one");
    assert_ne!(
        full, short,
        "a truncated stack must name its own frame on the interactive path too"
    );
    assert_eq!(
        state.renderer.frame_key(&cut, comp, 0, quality),
        Some(short),
        "and the same cut must name the same frame, or the cache is defeated"
    );
}

/// **The picture at a box** (docs/impl/node-graph-comp.md §4.5): the point
/// names a comp and a box, and the cut is a patched copy whose Output shows
/// that box.
///
/// Three things, each a way the chip could look as though it worked while
/// showing the wrong picture: the copy really rewires the Output; the box
/// that already feeds the Output cuts nothing, so the Viewer rides the
/// frame it has; and a point naming a comp with no graph cuts nothing
/// either.
#[test]
fn a_prefix_point_at_a_box_rewires_the_output() {
    use lumit_core::comp_graph::{CompGraph, GraphEdge, GraphNode};
    use lumit_core::model::{Composition, LinearColour, ProjectItem};
    use lumit_core::time::{Duration, FrameRate, Rational};

    let (project, layer_comp) = project_with_solid_of(4);
    let solid = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        match state
            .store
            .snapshot()
            .comp(layer_comp)
            .expect("comp")
            .layers[0]
            .kind
        {
            lumit_core::model::LayerKind::Solid { def } => def,
            _ => unreachable!("project_with_solid_of makes a solid layer"),
        }
    };

    // A graph of one Read into a blur into the Output, so the picture at
    // the Read is genuinely a different picture from the Output's.
    let read = Uuid::now_v7();
    let blur = lumit_core::fx::builtins::instantiate("blur").expect("a built-in");
    let blur_id = blur.id;
    let mut graph = CompGraph::new_with_output();
    let output = graph.output_id().expect("the seeded Output");
    graph.nodes.push(GraphNode::Read {
        id: read,
        item: solid,
        custom_name: None,
    });
    graph.nodes.push(GraphNode::Fx(blur));
    graph.edges.push(GraphEdge {
        from: read,
        from_port: "output".into(),
        to: blur_id,
        to_port: "input".into(),
    });
    graph.edges.push(GraphEdge {
        from: blur_id,
        from_port: "output".into(),
        to: output,
        to_port: "input".into(),
    });

    let comp = Uuid::now_v7();
    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(lumit_core::ops::Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(Composition {
                    graph: Some(graph),
                    master_volume_db: 0.0,
                    master_keys: None,
                    master_automation: Default::default(),
                    loudness_target: None,
                    lighting: Default::default(),
                    sound_mix: false,
                    groups: Vec::new(),
                    beat_grid: None,
                    id: comp,
                    name: "Graph".into(),
                    width: 64,
                    height: 32,
                    frame_rate: FrameRate::new(30, 1).expect("30 fps"),
                    duration: Duration(Rational::new(4, 30).expect("a duration")),
                    background: LinearColour([0.0, 0.0, 0.0, 0.0]),
                    work_area: None,
                    layers: Vec::new(),
                    markers: Vec::new(),
                    motion_blur: Default::default(),
                    extra: serde_json::Map::new(),
                })),
            })
            .expect("the node graph is filed");
    }
    let document = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot()
    };

    let at = |comp, node| crate::api::state::prefix::BridgePrefixPoint {
        layer: None,
        effect: None,
        graph: Some(crate::api::state::prefix::BridgeGraphPoint { comp, node }),
        shader_node: None,
    };
    let feeds = |doc: &std::sync::Arc<lumit_core::model::Document>| {
        doc.comp(comp)
            .expect("comp")
            .graph
            .as_ref()
            .expect("a node graph")
            .wire_into(output, "input")
            .map(|(from, _)| *from)
    };

    assert_eq!(feeds(&document), Some(blur_id), "as the document has it");
    assert_eq!(
        feeds(&super::viewed(Some(at(comp, read)), document.clone())),
        Some(read),
        "at the Read the Output shows the Read"
    );
    assert_eq!(
        feeds(&super::viewed(Some(at(comp, blur_id)), document.clone())),
        Some(blur_id),
        "the box that already feeds the Output cuts nothing"
    );
    assert_eq!(
        feeds(&super::viewed(Some(at(layer_comp, read)), document.clone())),
        Some(blur_id),
        "a point naming a comp with no graph cuts nothing"
    );
    assert_eq!(
        feeds(&super::viewed(None, document.clone())),
        Some(blur_id),
        "and the chip off is the document as it stands"
    );

    // And the cut names its own frame, which is the half a wrong cache
    // would get away with.
    let Some(mut state) = worker_state(project) else {
        return;
    };
    let quality = still_quality(1.0);
    let cut = super::viewed(Some(at(comp, read)), document.clone());
    let (whole, at_read) = (
        state.renderer.frame_key(&document, comp, 0, quality),
        state.renderer.frame_key(&cut, comp, 0, quality),
    );
    assert!(whole.is_some() && at_read.is_some(), "a solid is nameable");
    assert_ne!(
        whole, at_read,
        "the picture at a box must name its own frame, or the cache serves the Output's"
    );
}

/// **The picture at a box inside a Custom shader**: the stack stops at the
/// shader, and the copy's shader feeds its Result from the picked box. The
/// shader's own point leaves its graph as it was.
#[test]
fn a_prefix_point_inside_a_shader_feeds_its_result_from_the_box() {
    use crate::api::state::prefix::BridgePrefixPoint;
    use lumit_core::fx::effects::custom_shader::graph_of;

    let (project, comp) = project_with_solid_of(4);
    let layer_id = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot().comp(comp).expect("comp").layers[0].id
    };
    let mut shader = lumit_core::fx::builtins::instantiate("custom_shader").expect("a built-in");
    let graph = serde_json::json!({
        "nodes": [{ "id": 1, "kind": "result" }, { "id": 2, "kind": "uv" }],
    });
    shader
        .extra
        .insert("shader".into(), serde_json::json!({ "graph": graph }));
    let after = lumit_core::fx::builtins::instantiate("exposure").expect("a built-in");
    let document = {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(lumit_core::ops::Op::SetLayerEffects {
                comp,
                layer: layer_id,
                effects: vec![shader.clone(), after],
            })
            .expect("two effects");
        state.store.snapshot()
    };
    let at = |shader_node| BridgePrefixPoint {
        layer: Some(crate::api::layer::LayerReference::new(
            project.id, comp, layer_id,
        )),
        effect: Some(shader.id),
        graph: None,
        shader_node,
    };
    let stack = |doc: &std::sync::Arc<lumit_core::model::Document>| {
        doc.comp(comp).expect("comp").layers[0].effects.clone()
    };

    let cut = stack(&super::viewed(Some(at(Some(2))), document.clone()));
    assert_eq!(cut.len(), 1, "the stack stops at the shader");
    let edges = graph_of(&cut[0]).expect("a graph")["edges"].clone();
    assert_eq!(
        edges,
        serde_json::json!([{ "from": 2, "from_port": 0, "to": 1, "to_port": 0 }]),
        "the Result reads the uv box"
    );
    let own = stack(&super::viewed(Some(at(None)), document.clone()));
    assert_eq!(graph_of(&own[0]), Some(&graph), "the shader's own picture");
}
