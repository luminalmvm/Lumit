//! Tests for a transform's separate axes.

use super::*;

/// **Old projects load unchanged**. A transform written before
/// separate axes existed carries no `axis_modes` at all, and reads as the
/// default: both pairs combined, Scale linked.
#[test]
fn a_transform_without_axis_modes_reads_as_the_default() {
    let json = r#"{
            "anchor_x": {"animation": {"Static": 0.0}},
            "anchor_y": {"animation": {"Static": 0.0}},
            "position_x": {"animation": {"Static": 0.0}},
            "position_y": {"animation": {"Static": 0.0}},
            "scale_x": {"animation": {"Static": 100.0}},
            "scale_y": {"animation": {"Static": 100.0}},
            "rotation": {"animation": {"Static": 0.0}},
            "opacity": {"animation": {"Static": 100.0}}
        }"#;
    let group: TransformGroup = serde_json::from_str(json).unwrap();
    assert_eq!(group.axis_modes, AxisModes::default());
    assert_eq!(group.axis_modes.get(TransformPair::Scale), AxisMode::Linked);
    assert_eq!(
        group.axis_modes.get(TransformPair::Position),
        AxisMode::Combined
    );
}

/// An unknown key from a newer build survives a load and save beside the
/// new field (docs/03 §12's forward-compatibility rule).
#[test]
fn an_unknown_transform_field_survives_beside_the_axis_modes() {
    let mut group = TransformGroup::default();
    group
        .axis_modes
        .set(TransformPair::Anchor, AxisMode::Separated);
    let mut value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&group).unwrap()).unwrap();
    value["some_future_field"] = serde_json::json!(7);
    let back: TransformGroup = serde_json::from_value(value).unwrap();
    assert_eq!(back.axis_modes.anchor, AxisMode::Separated);
    assert_eq!(
        back.extra.get("some_future_field"),
        Some(&serde_json::json!(7))
    );
}

/// **Coming back together does not move the picture**. Two axes
/// keyed at different times gain each other's key times, and every axis
/// reads exactly what it read before at every moment.
#[test]
fn recombining_unions_the_key_times_without_changing_a_value() {
    use crate::anim::{Animation, Keyframe, SideInterp};
    let key = |t: i64, v: f64| Keyframe {
        time: Rational::new(t, 1).unwrap(),
        value: v,
        interp_in: SideInterp::Bezier {
            speed: 0.0,
            influence: 1.0 / 3.0,
        },
        interp_out: SideInterp::Bezier {
            speed: 0.0,
            influence: 1.0 / 3.0,
        },
    };
    let mut group = TransformGroup::default();
    group.position_x.animation = Animation::Keyframed(vec![key(0, 0.0), key(4, 100.0)]);
    group.position_y.animation = Animation::Keyframed(vec![key(1, 10.0), key(3, 50.0)]);

    let before: Vec<[f64; 2]> = (0..=40)
        .map(|i| {
            let t = f64::from(i) / 10.0;
            [group.position_x.value_at(t), group.position_y.value_at(t)]
        })
        .collect();

    let unified = group.unified_axes(TransformPair::Position);
    assert_eq!(unified.len(), 2, "both axes gained the other's times");
    for (prop, animation) in unified {
        group.get_mut(prop).unwrap().animation = animation;
    }

    let times = |a: &Animation| match a {
        Animation::Keyframed(keys) => keys.iter().map(|k| k.time.to_f64()).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(times(&group.position_x.animation), vec![0.0, 1.0, 3.0, 4.0]);
    assert_eq!(times(&group.position_y.animation), vec![0.0, 1.0, 3.0, 4.0]);

    for (i, expected) in before.iter().enumerate() {
        let t = f64::from(i as i32) / 10.0;
        let x = group.position_x.value_at(t);
        let y = group.position_y.value_at(t);
        assert!(
            (x - expected[0]).abs() < 1e-9,
            "x at {t}: {x} vs {}",
            expected[0]
        );
        assert!(
            (y - expected[1]).abs() < 1e-9,
            "y at {t}: {y} vs {}",
            expected[1]
        );
    }
}
