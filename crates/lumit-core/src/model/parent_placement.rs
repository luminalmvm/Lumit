//! Where a layer's parent puts it, for the placement chain.

use super::*;

/// `layer`'s placement as a parent: a solved camera is placed by its solve.
pub(super) fn parent_placement(
    comp: &Composition,
    layer: &Layer,
    t: f64,
    read: &Read<'_>,
    solves: Solves<'_>,
) -> Placement {
    let mut p = crate::orient::placed_in(comp, layer, t, read, solves);
    let solved = matches!(
        layer.kind,
        LayerKind::Camera {
            solve_link: Some(_),
            ..
        }
    );
    if let Some((doc, store)) = solves.filter(|_| solved) {
        if let Some(own) = crate::track::own_camera_pose_of(doc, comp, layer, t, store) {
            p.position = own.pose.position;
            p.rotation_deg = own.pose.rotation_deg;
        }
    }
    p
}
