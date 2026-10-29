//! A clip's own opacity, Vegas's event opacity: one more multiplier on its
//! picture, taken where its fades are.

use super::Clip;

pub(super) fn one() -> f64 {
    1.0
}

pub(super) fn is_one(v: &f64) -> bool {
    *v == 1.0
}

/// The clip's opacity held to nought to one. The bounds are constants, so
/// `clamp` has no reversed pair to panic on (docs/14 §4).
pub(super) fn opaque(clip: &Clip) -> f64 {
    clip.opacity.clamp(0.0, 1.0)
}

/// A dissolve's gain and the opacity its mix is drawn at, with each clip's
/// share weighed by its own opacity. Two whole clips keep the gain exactly as
/// it was, so a frame named before clips had an opacity keeps its name.
pub(super) fn weighed(outgoing: &Clip, incoming: &Clip, gain: f64) -> (f64, f64) {
    let (going, coming) = (opaque(outgoing), opaque(incoming));
    if going == 1.0 && coming == 1.0 {
        return (gain, 1.0);
    }
    let (going, coming) = ((1.0 - gain) * going, gain * coming);
    let sum = going + coming;
    if sum <= 0.0 {
        return (gain, 0.0);
    }
    (coming / sum, sum.min(1.0))
}
