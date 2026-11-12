//! Tests for applying a track to Position alone.

use super::*;

/// *Position* alone writes two properties and touches nothing else. The
/// point of the row: a small, plain patch gives a position worth having long
/// before it gives a rotation worth having, and a rotation of nought written
/// over one the user animated would be the button deleting their work.
#[test]
fn position_alone_leaves_rotation_and_scale_as_they_were() {
    use crate::model::TransformProp as P;

    let media = Uuid::now_v7();
    let shot = planar(layer("shot", LayerKind::Footage { item: media }, 2));
    let effect = shot.effects[0].id;
    let target = layer("logo", LayerKind::Null, 2);
    let (tracked_id, target_id) = (shot.id, target.id);
    let c = comp("main", vec![target, shot]);
    let comp_id = c.id;
    let mut doc = document(vec![c]);
    let store = MovingPlane { track: effect };

    let base_x = tr_value(&doc, comp_id, target_id, P::PositionX, 0.0);
    let op = transform_from_track(&doc, comp_id, tracked_id, effect, target_id, false, &store)
        .expect("position alone is still a track worth writing");
    apply(&mut doc, &op).unwrap();

    let moved = tr_value(&doc, comp_id, target_id, P::PositionX, 20.0 / 30.0);
    assert!(
        (moved - (base_x + 40.0)).abs() < 1e-6,
        "position still follows the track, got {moved}"
    );
    for prop in [P::Rotation, P::ScaleX, P::ScaleY] {
        assert!(
            is_static(&doc, comp_id, target_id, prop),
            "{prop:?} should not have been written"
        );
    }
}
