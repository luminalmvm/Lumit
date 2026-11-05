//! After Effects' parametric meshes as 3D shapes: the shape by the group its
//! rows sit in, and the rows with their keys.

use lumit_core::anim::Property as LumProperty;
use lumit_core::mesh_shape::MeshShape;
use lumit_core::model::{EffectInstance, EffectValue};

use super::props::{find, scalar, still};
use super::Conv;
use crate::capture::Property;
use crate::report::ItemPath;

type Rows = &'static [(&'static str, &'static str, f64)];

/// Each shape's group, and its rows: Lumit's id, After Effects' row and what
/// a missing row means.
const SHAPES: [(MeshShape, &str, Rows); 6] = [
    (
        MeshShape::Cube,
        "ADBE CubeMeshOptionsSGrp",
        &[
            ("width", "ADBE CubeWidthStrm", 200.0),
            ("height", "ADBE CubeHeightStrm", 200.0),
            ("depth", "ADBE CubeDepthStrm", 200.0),
            ("bevel", "ADBE CubeBevelRadiusStrm", 0.0),
            ("bevel_segments", "ADBE CubeBevelSidesStrm", 4.0),
        ],
    ),
    (
        MeshShape::Sphere,
        "ADBE SphereMeshOptionsSGrp",
        &[
            ("radius", "ADBE SphereRadiusStrm", 100.0),
            ("segments", "ADBE SphereSidesStrm", 48.0),
            ("slice_start", "ADBE SphereStartAngleStrm", 0.0),
            ("slice_end", "ADBE SphereEndAngleStrm", 360.0),
        ],
    ),
    (
        MeshShape::Cylinder,
        "ADBE CylinderMeshOptionsSGrp",
        &[
            ("radius", "ADBE CylinderRadiusStrm", 100.0),
            ("height", "ADBE CylinderHeightStrm", 200.0),
            ("segments", "ADBE CylinderSidesStrm", 48.0),
            ("bevel", "ADBE CylinderBevelRadiusStrm", 0.0),
            ("bevel_segments", "ADBE CylinderBevelSidesStrm", 4.0),
            ("slice_start", "ADBE CylinderStartAngleStrm", 0.0),
            ("slice_end", "ADBE CylinderEndAngleStrm", 360.0),
        ],
    ),
    (
        MeshShape::Cone,
        "ADBE ConeMeshOptionsSGrp",
        &[
            ("top_radius", "ADBE ConeTopRadiusStrm", 0.0),
            ("bottom_radius", "ADBE ConeBottomRadiusStrm", 100.0),
            ("height", "ADBE ConeHeightStrm", 200.0),
            ("segments", "ADBE ConeSidesStrm", 48.0),
            // Lumit bevels both rims alike, so the bottom's stands for both.
            ("bevel", "ADBE ConeBevelBottomRadiusStrm", 0.0),
            ("bevel_segments", "ADBE ConeBevelBottomSidesStrm", 4.0),
            ("slice_start", "ADBE ConeStartAngleStrm", 0.0),
            ("slice_end", "ADBE ConeEndAngleStrm", 360.0),
        ],
    ),
    (
        MeshShape::Torus,
        "ADBE TorusMeshOptionsSGrp",
        &[
            ("radius", "ADBE TorusRingRadiusStrm", 100.0),
            ("pipe_radius", "ADBE TorusPipeRadiusStrm", 30.0),
            ("segments", "ADBE TorusRingSidesStrm", 64.0),
            ("pipe_segments", "ADBE TorusPipeSidesStrm", 24.0),
            ("slice_start", "ADBE TorusStartAngleStrm", 0.0),
            ("slice_end", "ADBE TorusEndAngleStrm", 360.0),
        ],
    ),
    (
        MeshShape::Plane,
        "ADBE PlaneMeshOptionsSGrp",
        &[
            ("width", "ADBE PlaneWidthStrm", 400.0),
            ("height", "ADBE PlaneLengthStrm", 400.0),
            ("corner_radius", "ADBE PlaneCornerRadiusStrm", 0.0),
            ("corner_segments", "ADBE PlaneCornerSidesStrm", 8.0),
        ],
    ),
];

/// The shape a layer's rows say it is, if it is a parametric mesh.
pub(crate) fn shape_of(props: &[Property]) -> Option<MeshShape> {
    SHAPES
        .iter()
        .find(|(_, group, _)| find(props, group).is_some())
        .map(|(shape, ..)| *shape)
}

/// `shape`'s rows from the layer's, keys and all. An inverted slice keeps the
/// part a plain one cuts away, which is its two ends swapped.
pub(crate) fn rows(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    props: &[Property],
    shape: MeshShape,
) -> Vec<Option<EffectInstance>> {
    let Some((_, group, rows)) = SHAPES.iter().find(|(s, ..)| *s == shape) else {
        return Vec::new();
    };
    let Some(mut out) = lumit_core::fx::builtins::instantiate(shape.options()) else {
        return Vec::new();
    };
    let within = find(props, group).map_or(props, Property::children);
    let inverted = within
        .iter()
        .filter_map(|p| p.match_name.as_deref())
        .find(|m| m.ends_with("InvertSliceStrm"))
        .and_then(|m| still(within, m, 0))
        .is_some_and(|v| v != 0.0);
    for (id, name, fallback) in *rows {
        let id = match (*id, inverted) {
            ("slice_start", true) => "slice_end",
            ("slice_end", true) => "slice_start",
            (id, _) => id,
        };
        let value: LumProperty = scalar(conv, path, within, name, 0, *fallback);
        if let Some(row) = out.params.iter_mut().find(|p| p.id == id) {
            row.value = EffectValue::Float(value);
        }
    }
    vec![Some(out)]
}
