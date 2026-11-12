//! A stagger wired into Clone to points.

use super::*;

/// A Stagger wired into Clone to points' Rotation gives each
/// point of the stream its own value, through the same resolve the frame
/// uses, and a stream with nothing wired is left alone. Guards every stamp
/// taking the value its copy is owed.
#[test]
fn a_stagger_into_clone_to_points_turns_each_stamp() {
    use super::per_copy::{drive_rows, COPY_ROTATION, COPY_ROWS};
    let mut stagger = inst("stagger");
    set(&mut stagger, "maximum", 90.0);
    let target = inst("clone_to_points");
    let stream = || PointsStream {
        position: vec![[0.0; 3]; 3],
        speed: vec![[0.0; 3]; 3],
        age: vec![0.0; 3],
        life: vec![1.0; 3],
        size: vec![10.0; 3],
        rotation: vec![0.0; 3],
        colour: vec![[1.0; 4]; 3],
        id: vec![0, 1, 2],
        ..PointsStream::default()
    };

    let unwired = LayerGraph {
        nodes: vec![stagger.clone()],
        ..LayerGraph::default()
    };
    let mut s = stream();
    drive_rows(
        &unwired,
        target.id,
        &mut s,
        COPY_ROWS,
        0.0,
        &ctx(),
        None,
        points::Projection::FLAT,
    );
    assert!(s.named.is_empty(), "nothing wired, nothing written");

    let graph = LayerGraph {
        edges: vec![edge(
            &stagger,
            "value",
            NodeRef::Effect(target.id),
            "rotation",
        )],
        nodes: vec![stagger],
        ..LayerGraph::default()
    };
    let mut s = stream();
    drive_rows(
        &graph,
        target.id,
        &mut s,
        COPY_ROWS,
        0.0,
        &ctx(),
        None,
        points::Projection::FLAT,
    );
    let column = s.column(COPY_ROTATION);
    let values: Vec<f32> = (0..3).map(|i| s.read(column, i).unwrap()).collect();
    assert!((values[0] - 0.0).abs() < 1e-3, "{values:?}");
    assert!((values[1] - 45.0).abs() < 0.5, "{values:?}");
    assert!((values[2] - 90.0).abs() < 1e-3, "{values:?}");
}
