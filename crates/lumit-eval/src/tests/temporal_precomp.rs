//! A temporal effect on a precomp or an adjustment layer keys its neighbours.

use super::*;

/// **A temporal effect on a Precomp or an adjustment layer names its
/// neighbours.** Neither has source frames to stamp, so the picture a
/// frame away is named instead: without it, two times that show one
/// picture now and different ones next would share a name, and the cache
/// would hand one frame's blur to the other.
#[test]
fn a_temporal_effect_on_a_precomp_or_an_adjustment_keys_its_neighbours() {
    let mut doc = Document::new();
    // Still for five seconds and moving after: 4 s and 5 s show the same
    // picture, and only at 5 s is the next frame a different one.
    let key_at = |time: i64, value: f64| Keyframe {
        time: Rational::new(time, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let mut mover = text_layer("inner", 0.0, 10.0, 0.0);
    mover.transform.position_x = Property {
        animation: Animation::Keyframed(vec![key_at(0, 0.0), key_at(5, 0.0), key_at(10, 1000.0)]),
        extra: serde_json::Map::new(),
    };
    let blur = || lumit_core::fx::builtins::instantiate("motion_blur").unwrap();

    let nested = comp_with(vec![mover.clone()]);
    let mut pre = precomp_layer(nested.id);
    pre.effects.push(blur());
    doc.items.push(ProjectItem::Composition(nested));
    let mut adjust = text_layer("", 0.0, 10.0, 0.0);
    adjust.kind = LayerKind::Adjustment;
    adjust.effects.push(blur());

    for comp in [comp_with(vec![pre]), comp_with(vec![adjust, mover])] {
        assert_ne!(
            key(&doc, &comp, 4.0),
            key(&doc, &comp, 5.0),
            "one picture now, another next: two names"
        );
        assert_eq!(
            key(&doc, &comp, 3.0),
            key(&doc, &comp, 4.0),
            "and a still stretch keeps one name"
        );
    }

    // A Sequence layer's neighbour is whichever clip is live then. One
    // shot twice and then another: the last frame of each showing is the
    // same picture, and only the second is followed by the other shot.
    // Four frames a second, so every time here is exact.
    use lumit_core::sequence::{Clip, ClipSource};
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    let r = |n| Rational::new(n, 1).unwrap();
    let clip = |item, at| Clip::new(ClipSource::Footage(item), r(0), r(2), r(at), r(2));
    let mut cut = text_layer("", 0.0, 10.0, 0.0);
    cut.kind = LayerKind::Sequence {
        clips: vec![clip(a, 0), clip(a, 2), clip(b, 4)],
    };
    cut.effects.push(blur());
    let mut comp = comp_with(vec![cut]);
    comp.frame_rate = lumit_core::time::FrameRate::new(4, 1).unwrap();
    assert_ne!(
        key(&doc, &comp, 1.75),
        key(&doc, &comp, 3.75),
        "the same frame before two different clips: two names"
    );
    assert_eq!(
        key(&doc, &comp, 0.0),
        key(&doc, &comp, 2.0),
        "and the same frame before the same one keeps one name"
    );
}
