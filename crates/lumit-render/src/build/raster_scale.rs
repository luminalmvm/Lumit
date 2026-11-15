//! How much a shape layer's placement scales it, for its raster size.

/// How many comp pixels one of the layer's own pixels covers at most, read
/// off its world placement: the longest of the two axes' lengths, whichever
/// way round the matrix is laid out, so a turned or skewed layer is never
/// drawn short.
pub(super) fn shape_raster_scale(m: [[f32; 4]; 4]) -> f32 {
    let len = |a: f32, b: f32| a.hypot(b);
    len(m[0][0], m[1][0])
        .max(len(m[0][1], m[1][1]))
        .max(len(m[0][0], m[0][1]))
        .max(len(m[1][0], m[1][1]))
}
