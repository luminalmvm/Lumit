//! The Matte key's view sockets in a node graph.

use super::*;

/// The plate read into a Matte key with its View dropdown on `dropdown`, and
/// `rest` wiring the key's sockets on to the Output.
fn keyed_graph(
    dropdown: u32,
    rest: impl FnOnce(Uuid, Uuid, &mut Vec<GraphNode>, &mut Vec<GraphEdge>),
) -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let plate = Uuid::now_v7();
    doc.items.push(solid(plate, "plate", PLATE, COMP, COMP));
    let mut key = effect("matte_key", &[]);
    set_value(&mut key, "view", EffectValue::Choice(dropdown));
    let (src, key, out) = (read(plate), GraphNode::Fx(key), output());
    let (key_id, out_id) = (key.id(), out.id());
    let mut edges = vec![wire(src.id(), "output", key_id, "input")];
    let mut nodes = vec![src, key, out];
    rest(key_id, out_id, &mut nodes, &mut edges);
    let mut comp = graph_comp("graph", nodes, edges);
    comp.background = LinearColour([0.0; 4]);
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

/// **A view socket is the effect with its dropdown on that option.** The
/// Screen matte socket draws what `output` draws with the dropdown on Screen
/// matte, `output` goes on following the dropdown, and both can be wired at
/// once.
#[test]
fn a_view_socket_draws_the_view_it_is_named_for() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    const FINAL: u32 = 0;
    const SCREEN_MATTE: u32 = 1;
    let from = |dropdown: u32, port: &'static str| {
        keyed_graph(dropdown, move |key, out, _, edges| {
            edges.push(wire(key, port, out, "input"));
        })
    };
    let keyed = linear_at(&mut r, from(FINAL, "output"));
    let matte = linear_at(&mut r, from(SCREEN_MATTE, "output"));
    assert!(
        !near(keyed, matte),
        "the two views are different pictures: {keyed:?} and {matte:?}"
    );

    let socket = linear_at(&mut r, from(FINAL, "view_1"));
    assert!(
        near(socket, matte),
        "the Screen matte socket is the Screen matte view: {socket:?} against {matte:?}"
    );

    // Both wired at once, into a Switch that shows one or the other.
    let both = |index: f64| {
        keyed_graph(FINAL, move |key, out, nodes, edges| {
            let switch = GraphNode::Fx(effect("switch", &[("index", index)]));
            let switch_id = switch.id();
            nodes.push(switch);
            edges.push(wire(key, "output", switch_id, "in0"));
            edges.push(wire(key, "view_1", switch_id, "in1"));
            edges.push(wire(switch_id, "output", out, "input"));
        })
    };
    let first = linear_at(&mut r, both(0.0));
    let second = linear_at(&mut r, both(1.0));
    assert!(near(first, keyed), "{first:?} against {keyed:?}");
    assert!(near(second, matte), "{second:?} against {matte:?}");
}
