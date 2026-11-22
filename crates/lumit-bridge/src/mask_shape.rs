//! Comparing mask paths sent across from the app.

/// Whether two paths are the same shape, give or take a thousandth of a
/// pixel, which is rounding on the way across rather than an edit.
pub(crate) fn same_shape(
    a: &lumit_core::mask::BezierPath,
    b: &lumit_core::mask::BezierPath,
) -> bool {
    let near = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3;
    a.closed == b.closed
        && a.vertices.len() == b.vertices.len()
        && a.vertices.iter().zip(&b.vertices).all(|(p, q)| {
            near(p.pos, q.pos) && near(p.tan_in, q.tan_in) && near(p.tan_out, q.tan_out)
        })
}
