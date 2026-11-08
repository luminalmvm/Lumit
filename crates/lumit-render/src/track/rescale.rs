//! A track measured smaller than its source, put back into the source's pixels.

use lumit_track::planar::PlanarTrack;
use lumit_track::solve::CameraSolve;

/// [`rescale`], for a planar track: the corners are the whole of its geometry,
/// and they are pixels, so one multiply each is the whole conversion.
pub(super) fn rescale_planar(track: &mut PlanarTrack, scale: f64) {
    if !scale.is_finite() || scale <= 0.0 || (scale - 1.0).abs() < 1e-9 {
        return;
    }
    let f = 1.0 / scale;
    for corner in &mut track.reference_quad {
        corner[0] *= f;
        corner[1] *= f;
    }
    for frame in &mut track.frames {
        for corner in &mut frame.corners {
            corner[0] *= f;
            corner[1] *= f;
        }
    }
}

/// Put a solve measured at `scale` back into the source's own pixels.
///
/// A source rendered smaller than it is — a nested comp at the analysis raster —
/// gives a solve in *those* pixels: a focal in them, a world in them, an error
/// in them. Everything downstream reads the store as the source's own pixels
/// (§5b's second deviation), so the whole answer is multiplied by `1 / scale`
/// here, at the one place a solve is finished.
///
/// **Uniform, so the geometry is untouched.** The projection is
/// `focal · p.xy / p.z` with `p = R(P − C)`: multiplying the focal, the camera
/// centres and the world points all by the same number multiplies the projected
/// pixels by it too and changes nothing else — not a rotation, not a depth
/// ordering, not a reprojection *ratio*. It is a change of unit, not a fit.
///
/// The scaling is done after the solve rather than to the tracks before it,
/// because the solver's thresholds are in pixels: feeding it inflated
/// coordinates would silently tighten every one of them.
pub(super) fn rescale(solve: &mut CameraSolve, scale: f64) {
    if !scale.is_finite() || scale <= 0.0 || (scale - 1.0).abs() < 1e-9 {
        return;
    }
    let f = 1.0 / scale;
    for pose in &mut solve.poses {
        pose.focal_px *= f;
        pose.mean_reprojection_px *= f;
        for v in &mut pose.position {
            *v *= f;
        }
    }
    for segment in &mut solve.segments {
        segment.focal_px *= f;
    }
    for point in &mut solve.points {
        for v in &mut point.position {
            *v *= f;
        }
    }
    solve.mean_reprojection_px *= f;
}
