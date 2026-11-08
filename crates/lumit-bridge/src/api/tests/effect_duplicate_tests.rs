//! Duplicating effects in place.

use super::*;

/// The copy sits right after its original with the same keys and a name of
/// its own, and one undo takes it away again.
#[test]
fn a_duplicated_effect_follows_its_original_with_its_keys() {
    let (project, layer) = project_with_layer();
    layer.add_effect("blur".into()).expect("blur");
    layer.add_effect("glow".into()).expect("glow");
    let key = |num: i64, value: f64| BridgeKeyframe {
        time: BridgeRational { num, den: 1 },
        value,
        interp_in: BridgeSideInterp::Linear,
        interp_out: BridgeSideInterp::Linear,
    };
    let mut staged = layer.get_effects().expect("effects");
    staged[0]
        .set_value(
            "radius".into(),
            BridgeEffectValue::Float(BridgeScalar::Keyframed(vec![key(1, 0.0), key(2, 40.0)])),
        )
        .expect("animated");
    layer.set_effects(staged, None, None).expect("committed");
    let before = layer.get_effects().expect("effects");

    let made = layer
        .duplicate_effects(vec![before[0].id()])
        .expect("duplicated");

    let after = layer.get_effects().expect("effects");
    let ids: Vec<Uuid> = after.iter().map(|e| e.id()).collect();
    assert_eq!(ids, vec![before[0].id(), made[0], before[1].id()]);
    assert_eq!(
        after[1].get_value("radius".into()).expect("radius"),
        before[0].get_value("radius".into()).expect("radius"),
        "the copy keeps its keys where they were"
    );
    assert_eq!(
        after[1].get_info().custom_name.as_deref(),
        Some("Gaussian blur 2"),
        "the copy has a name of its own, so a link or an expression reaches it"
    );

    project.undo().expect("undone");
    assert_eq!(layer.get_effects().expect("effects").len(), 2);
}
