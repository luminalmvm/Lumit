//! Paint: strokes on a layer.

use super::*;

fn stroke(name: &str, points: &[(f64, f64)]) -> crate::api::layer::BridgeStroke {
    use crate::api::layer::{BridgePaintMode, BridgeStroke, BridgeStrokePoint};
    BridgeStroke {
        id: Uuid::now_v7(),
        name: name.into(),
        points: points
            .iter()
            .map(|&(x, y)| BridgeStrokePoint {
                x,
                y,
                pressure: 1.0,
            })
            .collect(),
        colour: crate::api::assets::BridgeColourRgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        width: 12.0,
        hardness: 0.8,
        shape: crate::api::layer::BridgeBrushShape::Round,
        opacity: 100.0,
        start: crate::api::effect::BridgeScalar::Static(0.0),
        end: crate::api::effect::BridgeScalar::Static(100.0),
        mode: BridgePaintMode::Paint,
        blend: 0,
        clone_offset_x: 0.0,
        clone_offset_y: 0.0,
    }
}

/// A brush drag is one stroke, one op and one undo step — which is what
/// `Ctrl+Z` after painting has to mean.
#[test]
fn a_stroke_is_added_read_back_and_undone_in_one_step() {
    let (project, layer) = project_with_layer();
    assert!(layer.get_paint().expect("paint").is_empty());

    layer
        .add_stroke(stroke("Brush 1", &[(10.0, 10.0), (40.0, 25.0)]))
        .expect("added");
    let strokes = layer.get_paint().expect("paint");
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].name, "Brush 1");
    assert_eq!(strokes[0].points.len(), 2);
    assert_eq!(strokes[0].points[1].x, 40.0);
    assert_eq!(strokes[0].width, 12.0);

    project.undo().expect("undone");
    assert!(
        layer.get_paint().expect("paint").is_empty(),
        "one stroke, one undo step"
    );
    project.redo().expect("redone");
    assert_eq!(layer.get_paint().expect("paint").len(), 1);
}
