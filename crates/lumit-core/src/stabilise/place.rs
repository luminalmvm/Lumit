//! Where a layer's pixels land on its comp's picture, as a matrix.
//!
//! In plain terms: the Viewer draws a stabilised layer's points over the
//! picture, and a Reverse puts one layer's shake back on another layer that
//! may sit, scale and turn differently. Both need the layer's own pixels in the
//! comp's, which is the compositor's placement: the same parent walk and
//! camera the render draws the layer with. A 2D layer's placement is exactly a
//! matrix; a 3D one's is the picture's stretch of it, which a stabiliser's
//! small moves never tell apart.

use std::sync::Arc;

use super::motion::{four_point, Mat3};
use crate::expression::ExpressionContext;
use crate::model::{Composition, Document, Layer};

/// `layer`'s own pixels to `comp`'s at comp time `t`. `None` when the layer
/// is placed edge on, with no area on the picture.
#[must_use]
pub fn layer_to_comp(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    t: f64,
) -> Option<Mat3> {
    let context = ExpressionContext {
        document: Arc::clone(doc),
        comp: Some(comp.id),
        layer: Some(layer.id),
        comp_time: t,
        current_depth: 0,
        inputs: None,
    };
    let read = crate::expression::space::reader(&context, comp.id, t);
    let space = crate::expression::space::space(doc, comp, layer, t, &read);
    // Any four points in general position settle the map; a square a hundred
    // pixels across keeps the numbers well scaled.
    let probe = [[0.0, 0.0], [100.0, 0.0], [0.0, 100.0], [100.0, 100.0]];
    let to = probe.map(|p| {
        let q = space.to_comp((p[0], p[1], 0.0));
        [q.0, q.1]
    });
    four_point(probe, to)
}
