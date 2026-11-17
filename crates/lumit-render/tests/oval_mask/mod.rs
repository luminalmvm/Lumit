//! The oval mask the mask-path effects draw along.

use super::*;

/// An oval covering most of the frame, for the three effects that draw along a
/// mask path (Vegas, Scribble, Stroke). Four vertices with the usual circular
/// tangent length, which is close enough to an ellipse for a figure.
pub(super) fn oval_mask(mode: MaskMode) -> Mask {
    let (cx, cy) = (f64::from(W) / 2.0, f64::from(H) / 2.0);
    let (rx, ry) = (f64::from(W) * 0.3, f64::from(H) * 0.34);
    // The magic number that turns four cubic segments into a circle.
    let k = 0.552_284_75;
    let (hx, hy) = (rx * k, ry * k);
    let v = |pos: (f64, f64), tin: (f64, f64), tout: (f64, f64)| Vertex {
        pos,
        tan_in: tin,
        tan_out: tout,
    };
    Mask {
        id: id("Oval"),
        name: "Oval".into(),
        path: BezierPath {
            vertices: vec![
                v((cx, cy - ry), (-hx, 0.0), (hx, 0.0)),
                v((cx + rx, cy), (0.0, -hy), (0.0, hy)),
                v((cx, cy + ry), (hx, 0.0), (-hx, 0.0)),
                v((cx - rx, cy), (0.0, hy), (0.0, -hy)),
            ],
            closed: true,
        },
        path_keys: Vec::new(),
        inverted: false,
        opacity: Property::fixed(100.0),
        mode,
        feather: Property::zero(),
        vertex_feather: Vec::new(),
        expansion: Property::zero(),
        extra: serde_json::Map::new(),
    }
}
