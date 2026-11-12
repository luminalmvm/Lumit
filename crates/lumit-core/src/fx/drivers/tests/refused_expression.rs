//! Tests for an expression the engine refuses.

use super::*;

/// A refused expression pushes nothing, and the parameter it is wired to
/// resolves to its own stored value — the same calm the walk gives a
/// bypassed driver or a spent budget. Every kind of refusal: a runtime
/// error, a syntax error, a name not in scope, a result of the wrong
/// shape, an infinity, and no text at all.
#[test]
fn a_refused_expression_pushes_nothing_and_the_wired_parameter_keeps_its_keyframes() {
    let mut blur = inst("blur");
    set(&mut blur, "radius", 40.0);
    let radius = |source: &str| -> f32 {
        let e = expression(source);
        let graph = LayerGraph {
            edges: vec![edge(&e, "value", NodeRef::Effect(blur.id), "radius")],
            nodes: vec![e],
            ..LayerGraph::default()
        };
        let drivers = resolve_drivers(&graph, 0.5, ctx(), None);
        assert!(drivers.is_empty(), "{source:?} carried something");
        crate::fx::resolved::resolve_stack_temporal_named(
            std::slice::from_ref(&blur),
            &drivers,
            0.5,
            0.5,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            ctx(),
        )
        .1
        .get(0)
        .expect("one op")
        .params
        .float(ParamId::new("radius"), -1.0)
    };
    for refused in [
        "1 / 0",
        "1 +",
        "no_such_name * 2",
        "\"words\"",
        "[1]",
        "[1, 2, 3, 4, 5]",
        "1.0 / 0.0",
        "",
        "   ",
    ] {
        assert_eq!(radius(refused), 40.0, "{refused:?}: the stored keyframe");
    }
}
