//! Tests for moving a point between a layer's spaces.

use super::*;

/// **A point crosses between a layer's spaces and back, in both
/// languages.** On the picture a 2D layer is its flat placement, a 3D one
/// is that placement seen through the camera, and every way there has a
/// way back that lands on the point it started from.
#[test]
fn a_point_crosses_a_layers_spaces_and_back_in_both_languages() {
    let (document, comp, names) = staged();
    let on = |name: &str| {
        let layer = names.iter().find(|(n, _)| n == name).map(|(_, id)| *id);
        Arc::new(ExpressionContext {
            document: document.clone(),
            comp: Some(comp),
            layer,
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        })
    };
    let js = |source: &str| evaluate_in(Language::JavaScript, source, Some(on("Flat")));
    let rhai = |source: &str| evaluate_in(Language::Rhai, source, Some(on("Flat")));

    // The 2D layer is never seen through the camera, and keeps as many
    // numbers as it was given.
    assert_eq!(js("toComp([5, 5])[0] * 1000 + toComp([5, 5])[1]"), 90_055.0);
    assert_eq!(
        js("toComp([5, 5, 7]).length * 10 + toComp([5, 5, 7])[2]"),
        30.0
    );
    assert_eq!(js("fromComp([90, 55])[0] + fromComp([90, 55])[1]"), 10.0);
    assert_eq!(rhai("layer().to_comp([5.0, 5.0])[0]"), 90.0);
    // The 3D one with the same transform is in the same place in the
    // comp's space, and on the picture where the camera puts that.
    assert_eq!(js("thisComp.layer('Plate').toWorld([5, 5])[0]"), 90.0);
    let pose = document.comp(comp).and_then(|c| c.camera_pose(0.0));
    let (x, y, depth) =
        crate::camera::project_depth(&pose.expect("a camera"), 1920.0, 1080.0, (90.0, 55.0, 0.0));
    for (axis, want) in [x, y, depth].into_iter().enumerate() {
        let got = js(&format!(
            "thisComp.layer('Plate').toComp([5, 5, 0])[{axis}]"
        ));
        assert!((got - want).abs() < 1e-9, "{axis}: {got} against {want}");
    }

    // Every way and its way back, at a time given and at the time now.
    // Not toCompVec through a camera: After Effects' fromCompVec isn't
    // quite its inverse, and the render tests hold both to its numbers.
    let ways = [
        ("toComp", "fromComp", "to_comp", "from_comp"),
        ("toWorld", "fromWorld", "to_world", "from_world"),
        (
            "toWorldVec",
            "fromWorldVec",
            "to_world_vec",
            "from_world_vec",
        ),
    ];
    for (to, from, to_r, from_r) in ways {
        let off = js(&format!(
            "var L = thisComp.layer('Card'); var q = L.{from}(L.{to}([3, 4, 5], 1)); \
                 1000 + Math.abs(q[0] - 3) + Math.abs(q[1] - 4) + Math.abs(q[2] - 5)"
        ));
        assert!(
            (off - 1000.0).abs() < 1e-6,
            "{to} and {from} in JavaScript: {off}"
        );
        // Rhai takes an expression rather than statements, so the
        // card is named each time.
        let card = "layer(\"Card\")";
        for back in [
            format!("{card}.{from_r}({card}.{to_r}([3.0, 4.0, 5.0]), 1.0)"),
            format!("{card}.{from_r}({card}.{to_r}([3.0, 4.0, 5.0], 1.0))"),
        ] {
            let off = rhai(&format!(
                "1000.0 + abs({back}[0] - 3.0) + abs({back}[1] - 4.0) + abs({back}[2] - 5.0)"
            ));
            assert!((off - 1000.0).abs() < 1e-6, "{back} in Rhai: {off}");
        }
    }
    // A point on the picture meets the card's own plane where it showed.
    let off = js(
        "var L = thisComp.layer('Card'); var q = L.fromCompToSurface(L.toComp([7, 8, 0])); \
             q.length * 100 + Math.abs(q[0] - 7) + Math.abs(q[1] - 8)",
    );
    assert!((off - 200.0).abs() < 1e-6, "{off}");
    let back = "layer(\"Card\").from_comp_to_surface(layer(\"Card\").to_comp([7.0, 8.0, 0.0]))";
    let off = rhai(&format!(
        "1000.0 + abs({back}[0] - 7.0) + abs({back}[1] - 8.0)"
    ));
    assert!((off - 1000.0).abs() < 1e-6, "{off}");
    let at = rhai("layer(\"Card\").from_comp_to_surface([960.0, 540.0], 1.0).len()");
    assert_eq!(at, 2.0);

    // A 3D layer's points have a z, its turn is in the three rotations,
    // and the camera answers for its own rows.
    let zoom = crate::camera::default_zoom(1920.0);
    assert_eq!(js("thisComp.layer('Card').position.length"), 3.0);
    assert_eq!(js("thisComp.layer('Card').transform.position[2]"), 200.0);
    assert_eq!(
        js("thisComp.layer('Card').anchorPoint[2] + thisComp.layer('Card').scale[2]"),
        100.0
    );
    assert_eq!(
        js("thisComp.layer('Null').yRotation + thisComp.layer('Card').zRotation"),
        45.0
    );
    assert_eq!(
        js("thisComp.layer('Camera').xRotation + thisComp.layer('Card').orientation[1]"),
        -5.0
    );
    assert_eq!(js("thisComp.activeCamera.zoom"), zoom);
    assert_eq!(
        js("thisComp.activeCamera.cameraOption.pointOfInterest[0]"),
        960.0
    );
    assert_eq!(
        rhai("layer(\"Card\").z + layer(\"Null\").rotation_y + layer(\"Card\").rotation_z"),
        245.0
    );
    assert_eq!(
        rhai("layer(\"Camera\").rotation_x + layer(\"Card\").orientation[2]"),
        -5.0
    );
    assert_eq!(rhai("comp().active_camera.zoom"), zoom);
    assert_eq!(rhai("comp().active_camera.point_of_interest[1]"), 540.0);
    assert_eq!(rhai("layer(\"Card\").zoom"), -1.0, "a card has no zoom");
}
