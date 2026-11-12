//! A Stabilise written down as the layer's own keyframes.
//!
//! In plain terms: the effect moves the picture inside the layer, a little
//! every frame. The same move can be made by the layer's transform instead, by
//! shifting its Anchor point and adding to its Scale and Rotation, which is
//! what other editors' stabilisers write. Done that way the motion is ordinary
//! keyframes: it shows in the graph editor, copies, and can be parented to.
//!
//! The effect draws output pixel `p` from input pixel `W p`, so the picture is
//! moved by `A = W⁻¹` in the layer's own pixels, before the layer's transform
//! `T = Translate(position) · Rotate(rotation) · Scale(scale) · Translate(−anchor)`.
//! For a similarity `A(q) = s · R(θ) · q + d`, `T · A` is again a layer
//! transform: the anchor becomes `W(anchor)`, the rotation gains `θ`, the scale
//! is multiplied by `s`, and the position stays. That is exact with a uniform
//! scale. With a non-uniform scale and a turn the scale and the turn no longer
//! commute, and this is the usual approximation.

use std::sync::Arc;

use uuid::Uuid;

use super::motion::{apply, Mat3};
use super::resolve::{resolve, takes_out};
use super::rows::Reader;
use super::shown::nearest;
use crate::anim::{Animation, Keyframe, Property, SideInterp};
use crate::expression::ExpressionContext;
use crate::fx::effects::stabilise::planned;
use crate::model::{Document, TransformProp};
use crate::ops::Op;
use crate::time::Rational;

/// The three parts of a layer transform a stabilisation is written onto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moved {
    /// Layer pixels.
    pub anchor: [f64; 2],
    /// Percent, 100 for natural size.
    pub scale: [f64; 2],
    /// Degrees.
    pub rotation: f64,
}

/// The similarity nearest `w` over a `size` frame's four corners, by least
/// squares, as `(a, b, tx, ty)` for `[[a, −b, tx], [b, a, ty]]`. Exact when
/// `w` is already one; the nearest turn, scale and slide for a perspective
/// map, which a layer transform cannot carry. `None` past `w`'s horizon.
fn nearest_similarity(w: &Mat3, size: [f64; 2]) -> Option<[f64; 4]> {
    let corners = [
        [0.0, 0.0],
        [size[0], 0.0],
        [0.0, size[1]],
        [size[0], size[1]],
    ];
    let mut to = [[0.0; 2]; 4];
    for (q, c) in to.iter_mut().zip(corners) {
        *q = apply(w, c)?;
    }
    let mean = |pts: &[[f64; 2]; 4]| {
        let s = pts
            .iter()
            .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
        [s[0] / 4.0, s[1] / 4.0]
    };
    let (pm, qm) = (mean(&corners), mean(&to));
    let (mut dot, mut cross, mut len) = (0.0, 0.0, 0.0);
    for (p, q) in corners.iter().zip(&to) {
        let (px, py) = (p[0] - pm[0], p[1] - pm[1]);
        let (qx, qy) = (q[0] - qm[0], q[1] - qm[1]);
        dot += px * qx + py * qy;
        cross += px * qy - py * qx;
        len += px * px + py * py;
    }
    if len < 1e-12 {
        return None;
    }
    let (a, b) = (dot / len, cross / len);
    let s = [
        a,
        b,
        qm[0] - (a * pm[0] - b * pm[1]),
        qm[1] - (b * pm[0] + a * pm[1]),
    ];
    s.iter().all(|v| v.is_finite()).then_some(s)
}

/// The layer transform that draws what `before` draws with the picture moved
/// as an effect drawing with `w` moves it, `w` being the map from an output
/// pixel to the input pixel it shows, in a `size` frame of the layer's own
/// pixels. `near` is the turn the frame before came to, so a turn across ±180
/// degrees carries on rather than jumping. Also answers the turn this frame
/// adds, for the next frame's `near`.
#[must_use]
pub fn moved(w: &Mat3, size: [f64; 2], before: Moved, near: f64) -> Option<(Moved, f64)> {
    let fit = nearest_similarity(w, size)?;
    let [a, b, _, _] = fit;
    let s = a.hypot(b);
    if s < 1e-9 {
        return None;
    }
    // `w` runs the picture's move backwards: its turn and its scale are the
    // move's undone.
    let raw = -b.atan2(a).to_degrees();
    let turn = near + (raw - near + 180.0).rem_euclid(360.0) - 180.0;
    let anchor = apply(&super::motion::from_similarity(fit), before.anchor)?;
    Some((
        Moved {
            anchor,
            scale: [before.scale[0] / s, before.scale[1] / s],
            rotation: before.rotation + turn,
        },
        turn,
    ))
}

/// One baked key: linear on both sides, one a frame.
fn key(time: Rational, value: f64) -> Keyframe {
    Keyframe {
        time,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

/// `baked` with the keys `own` already had before its first key and after
/// its last kept beside it, in time order: the property's animation outside
/// the span the analysis covers is left as it was.
fn kept_with(own: &Property, mut baked: Vec<Keyframe>) -> Vec<Keyframe> {
    let (Animation::Keyframed(had), Some(lo), Some(hi)) = (
        &own.animation,
        baked.first().map(|k| k.time),
        baked.last().map(|k| k.time),
    ) else {
        return baked;
    };
    baked.extend(had.iter().filter(|k| k.time < lo || k.time > hi).copied());
    baked.sort_by_key(|k| k.time);
    baked
}

/// **Convert to keyframes**: the Stabilise `effect` on `layer` of `comp`
/// written onto the layer's Anchor point, Scale and Rotation, and the effect
/// switched off, as one undo step. Answers the step and how many keys each
/// property has.
///
/// One linear key per comp frame the layer shows that the analysis covers, at
/// the comp's frame times, with the map each frame is drawn with: Amount and
/// Extra scale on top, the Crop and scale zoom in it. Each key is composed with
/// what the property held at that frame. The property's own keys before and
/// after that span are kept; inside it the baked keys replace them. A 3D layer's Z parts are left as they are. Crop, Fill and the
/// rolling shutter cannot be carried by a transform: the picture shows its
/// moved edges, and the shutter's lean stays in it.
///
/// `None` when the effect is not a switched-on Stabilise taking the shake out,
/// when nothing is analysed for what the layer shows, or when no frame of the
/// layer falls in the analysis.
#[must_use]
pub fn bake(doc: &Arc<Document>, comp: Uuid, layer: Uuid, effect: Uuid) -> Option<(Op, u32)> {
    let c = doc.comp(comp)?;
    let l = c.layers.iter().find(|l| l.id == layer)?;
    let inst = l
        .effects
        .iter()
        .find(|e| e.id == effect)
        .filter(|e| takes_out(e))?;
    let reader = Reader {
        doc,
        comp: c,
        layer: l,
        inst,
    };
    let p = planned(&reader)?;
    let span = f64::from(p.filed.analysis.frames) - 1.0;

    let rate = c.frame_rate;
    let first = rate.frame_at(l.in_point);
    let last = rate.frame_at(l.out_point);
    let tr = &l.transform;
    let mut keys: [Vec<Keyframe>; 5] = Default::default();
    let mut near = 0.0;
    for n in first..last {
        let ct = rate.time_of_frame(n).ok()?;
        let t = ct.0.to_f64();
        let lt = crate::time::layer_time(t, l.start_offset.0);
        // The analysed frame this one shows, through any Retime. Past either
        // end the effect holds the end frame, which is no motion to write.
        let at = nearest(l, lt, p.rate, &p.filed.analysis);
        if !(0.0..=span).contains(&at) {
            continue;
        }
        // The map exactly as the frame is drawn with.
        let Some(drawn) = resolve(&reader, lt) else {
            continue;
        };
        // What the three properties hold here, keys and expressions read as
        // the renderer reads them.
        let context = ExpressionContext {
            document: Arc::clone(doc),
            comp: Some(c.id),
            layer: Some(l.id),
            comp_time: t,
            current_depth: 0,
            inputs: None,
        };
        let read = crate::expression::space::reader(&context, c.id, t);
        let before = Moved {
            anchor: [read(l, &tr.anchor_x), read(l, &tr.anchor_y)],
            scale: [read(l, &tr.scale_x), read(l, &tr.scale_y)],
            rotation: read(l, &tr.rotation),
        };
        let Some((after, turn)) = moved(&drawn.map, drawn.size, before, near) else {
            continue;
        };
        near = turn;
        // Keyframe times are layer time, which every property is read at.
        let time = ct.0.checked_sub(l.start_offset.0).ok()?;
        for (slot, value) in keys.iter_mut().zip([
            after.anchor[0],
            after.anchor[1],
            after.scale[0],
            after.scale[1],
            after.rotation,
        ]) {
            slot.push(key(time, value));
        }
    }
    let count = u32::try_from(keys[0].len()).ok()?;
    if count == 0 {
        return None;
    }

    let props = [
        TransformProp::AnchorX,
        TransformProp::AnchorY,
        TransformProp::ScaleX,
        TransformProp::ScaleY,
        TransformProp::Rotation,
    ];
    let own = [
        &tr.anchor_x,
        &tr.anchor_y,
        &tr.scale_x,
        &tr.scale_y,
        &tr.rotation,
    ];
    let mut ops: Vec<Op> = props
        .into_iter()
        .zip(keys)
        .zip(own)
        .map(|((prop, keys), own)| Op::SetTransformProperty {
            comp,
            layer,
            prop,
            animation: Animation::Keyframed(kept_with(own, keys)),
        })
        .collect();
    // Switched off, not removed: switching it back on and taking the keys
    // away is the effect as it was.
    let effects = l
        .effects
        .iter()
        .map(|e| {
            let mut e = e.clone();
            if e.id == effect {
                e.enabled = false;
            }
            e
        })
        .collect();
    ops.push(Op::SetLayerEffects {
        comp,
        layer,
        effects,
    });
    Some((Op::Batch { ops }, count))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::camera::place;
    use crate::stabilise::motion::{from_similarity, invert};

    /// The baked transform must draw every layer pixel where the transform and
    /// the effect together drew it, or the picture moves when the bake lands.
    #[test]
    fn baked_transform_draws_the_picture_where_the_effect_did() {
        let size = [1920.0, 1080.0];
        // The picture moved by a turn of 7 degrees, grown by a tenth and slid.
        let (s, th) = (1.1f64, 7f64.to_radians());
        let a = from_similarity([s * th.cos(), s * th.sin(), 12.0, -5.0]);
        let w = invert(&a).expect("invertible");
        let before = Moved {
            anchor: [300.0, 200.0],
            scale: [80.0, 80.0],
            rotation: 20.0,
        };
        let position = (960.0, 540.0, 0.0);
        let (after, _) = moved(&w, size, before, 0.0).expect("moved");
        let t = place(
            position,
            (before.anchor[0], before.anchor[1], 0.0),
            (before.scale[0], before.scale[1], 100.0),
            (0.0, 0.0, before.rotation),
        );
        let t2 = place(
            position,
            (after.anchor[0], after.anchor[1], 0.0),
            (after.scale[0], after.scale[1], 100.0),
            (0.0, 0.0, after.rotation),
        );
        for q in [[0.0, 0.0], [1920.0, 0.0], [640.0, 900.0], [-50.0, 333.0]] {
            let aq = apply(&a, q).expect("finite");
            let want = t.point((aq[0], aq[1], 0.0));
            let got = t2.point((q[0], q[1], 0.0));
            assert!(
                (want.0 - got.0).abs() < 1e-6 && (want.1 - got.1).abs() < 1e-6,
                "{q:?}: {want:?} against {got:?}"
            );
        }
    }
}
