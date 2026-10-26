//! Masks: each one's mode, feather and path, keys and all.

use lumit_core::anim::Property as LumProperty;
use lumit_core::mask::{BezierPath, Mask, MaskMode, PathKeyframe, Vertex};
use uuid::Uuid;

use crate::capture::{Property, Shape};
use crate::report::{ItemPath, Outcome, Reason};

use super::props::{ae_map, child, display_name, group, match_name_of, scalar, still};
use super::Conv;

/// Every mask on the layer, in AE's own order.
pub(super) fn masks(conv: &mut Conv<'_>, path: &ItemPath, props: &[Property]) -> Vec<Mask> {
    group(props, "ADBE Mask Parade")
        .iter()
        .filter_map(|node| mask(conv, path, node))
        .collect()
}

fn mask(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<Mask> {
    let facts = node.mask.clone()?;
    let name = display_name(node, match_name_of(node)).to_string();
    let here = path.property(&name);
    let props = node.children();

    let mode = match facts.mode.as_deref() {
        Some("NONE") => MaskMode::None,
        Some("SUBTRACT") => MaskMode::Subtract,
        Some("INTERSECT") => MaskMode::Intersect,
        Some("DIFFERENCE") => MaskMode::Difference,
        Some("LIGHTEN") => MaskMode::Lighten,
        Some("DARKEN") => MaskMode::Darken,
        _ => MaskMode::Add,
    };

    if facts.roto_bezier == Some(true) {
        conv.report.row(
            here.clone(),
            Outcome::Adjusted,
            Reason::MaskRotoBezierFlattened,
        );
    }

    // AE feathers separately in x and y; Lumit has one width.
    let (fx, fy) = (
        still(props, "ADBE Mask Feather", 0).unwrap_or(0.0),
        still(props, "ADBE Mask Feather", 1).unwrap_or(0.0),
    );
    if (fx - fy).abs() > 1e-9 {
        conv.report.row(
            here.clone(),
            Outcome::Adjusted,
            Reason::MaskFeatherAxesDiffer { x: fx, y: fy },
        );
    }

    let shape = child(props, "ADBE Mask Shape");
    let path_keys: Vec<PathKeyframe> = shape
        .and_then(|node| node.keyframes.as_deref())
        .map(|keys| {
            keys.iter()
                .filter_map(|key| {
                    let shape: Shape = serde_json::from_value(key.v.clone()?).ok()?;
                    Some(PathKeyframe {
                        time: conv.layer_time(key.t?),
                        path: bezier(&shape),
                        interp_in: super::props::path_side(key.in_interp.as_deref()),
                        interp_out: super::props::path_side(key.out_interp.as_deref()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let still_path = shape
        .and_then(crate::capture::Property::shape)
        .as_ref()
        .map(bezier)
        .or_else(|| path_keys.first().map(|k| k.path.clone()))
        .unwrap_or(BezierPath {
            vertices: Vec::new(),
            closed: true,
        });

    conv.report.imported();
    Some(Mask {
        id: Uuid::now_v7(),
        name,
        path: still_path,
        path_keys,
        inverted: facts.inverted.unwrap_or(false),
        opacity: scalar(conv, &here, props, "ADBE Mask Opacity", 0, 100.0),
        mode,
        feather: LumProperty::fixed((fx + fy) / 2.0),
        // AE's variable feather is a second point set with positions of its
        // own, which Lumit's per-vertex widths are not a place to put:
        // no fixture proves the layout, and guessing one would draw a shape
        // nobody asked for. The single width above stands, as it always has.
        vertex_feather: Vec::new(),
        expansion: scalar(conv, &here, props, "ADBE Mask Offset", 0, 0.0),
        extra: ae_map(vec![
            ("mode", serde_json::json!(facts.mode)),
            ("feather", serde_json::json!([fx, fy])),
            ("colour", serde_json::json!(facts.colour)),
        ]),
    })
}

/// A capture path as Lumit's own — parallel arrays become one vertex each.
pub(super) fn bezier(shape: &Shape) -> BezierPath {
    let point = |list: &[Vec<f64>], i: usize| {
        list.get(i).map_or((0.0, 0.0), |p| {
            (
                p.first().copied().unwrap_or(0.0),
                p.get(1).copied().unwrap_or(0.0),
            )
        })
    };
    BezierPath {
        vertices: (0..shape.vertices.len())
            .map(|i| Vertex {
                pos: point(&shape.vertices, i),
                tan_in: point(&shape.in_tangents, i),
                tan_out: point(&shape.out_tangents, i),
            })
            .collect(),
        closed: shape.closed.unwrap_or(true),
    }
}
