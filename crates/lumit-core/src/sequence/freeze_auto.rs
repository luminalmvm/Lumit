//! Freezing the automatic tangents either side of a cut in a map.

use super::*;

/// Write down the **automatic** tangents either side of `tau` as the beziers
/// they currently resolve to, so a cut there can keep the curve.
///
/// In plain terms: an automatic tangent does not store a direction, it works
/// one out from the keys on either side of it. A cut changes exactly
/// those neighbours — the key before the cut gains the cut as its new
/// neighbour, and the key after it ends up first in a clip of its own with
/// nothing to its left — so an automatic side would quietly re-aim itself and
/// the two halves would stop playing what the whole clip played. Said out loud
/// as the bezier it already was, the tangent cannot drift.
///
/// Only the pair the cut lands between is touched: every other key goes on
/// aiming itself, because nothing about its neighbours changed. An automatic
/// speed depends on its neighbours' *times and values*, never on their sides,
/// so writing these two down does not disturb the ones left automatic.
///
/// A no-op when the property is not keyframed, or when `tau` is outside the
/// keyed range — there is no span for the cut to land in.
pub(super) fn freeze_auto_around(map: &mut Property, tau: Rational) {
    let Animation::Keyframed(keys) = &mut map.animation else {
        return;
    };
    let t = tau.to_f64();
    let Some(i) = keys
        .windows(2)
        .position(|w| t > w[0].time.to_f64() && t < w[1].time.to_f64())
    else {
        return;
    };
    // Both keys read before either is written: each side is resolved against
    // the map as it stands, not against a half-frozen one.
    let sides = [
        (
            crate::anim::resolved_side(keys, i, false),
            crate::anim::resolved_side(keys, i, true),
        ),
        (
            crate::anim::resolved_side(keys, i + 1, false),
            crate::anim::resolved_side(keys, i + 1, true),
        ),
    ];
    for (offset, (interp_in, interp_out)) in sides.into_iter().enumerate() {
        keys[i + offset].interp_in = interp_in;
        keys[i + offset].interp_out = interp_out;
    }
}
