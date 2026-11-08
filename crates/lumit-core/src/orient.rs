//! A layer that turns by itself: along the way it is moving, or towards the
//! camera. The turn goes outside the layer's own Rotation and Orientation, so
//! those still add to it, and it goes into the placement, so what is parented
//! to the layer and its motion blur both see it.
//!
//! Off is every file from before this existed, and writes nothing.

use serde::{Deserialize, Serialize};

use crate::anim::{Animation, Property};
use crate::camera::{self, Mat3};
use crate::model::{
    Composition, Document, Layer, LayerKind, Placement, Read, Solves, TransformGroup,
};
use crate::ops::{Op, OpError};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orient {
    #[default]
    Off,
    AlongPath,
    TowardsCamera,
}

impl Orient {
    /// For serde, and for the places that only care if there is a turn at all.
    #[must_use]
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde hands a reference
    pub fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }
}

/// Set a layer's Orient, handing back the op that puts it back.
pub(crate) fn set(
    doc: &mut Document,
    comp: Uuid,
    layer: Uuid,
    orient: Orient,
) -> Result<Op, OpError> {
    let c = doc.comp_mut(comp).ok_or(OpError::UnknownComp)?;
    let l = c
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .ok_or(OpError::UnknownLayer)?;
    let previous = std::mem::replace(&mut l.transform.orient, orient);
    Ok(Op::SetLayerOrient {
        comp,
        layer,
        orient: previous,
    })
}

/// Half the stretch of time a direction is measured over, in seconds. Well
/// under a frame at any rate, and the same for preview and export.
const STEP: f64 = 1.0 / 1000.0;

/// Which way Position is heading at layer time `lt`, or `None` on a layer that
/// never moves. A layer stopped on its path keeps pointing the way it last
/// went, and one yet to start points the way it is about to go.
#[must_use]
pub fn direction(tr: &TransformGroup, lt: f64, flat: bool) -> Option<(f64, f64, f64)> {
    let at = |t: f64| {
        (
            tr.position_x.value_at(t),
            tr.position_y.value_at(t),
            if flat { 0.0 } else { tr.position_z.value_at(t) },
        )
    };
    let heading = |a: f64, b: f64| {
        let (p, q) = (at(a), at(b));
        let d = (q.0 - p.0, q.1 - p.1, q.2 - p.2);
        (d.0 * d.0 + d.1 * d.1 + d.2 * d.2 > 1e-18).then_some(d)
    };
    if let Some(d) = heading(lt - STEP, lt + STEP) {
        return Some(d);
    }
    let mut times: Vec<f64> = [&tr.position_x, &tr.position_y, &tr.position_z]
        .into_iter()
        .filter_map(|p: &Property| match &p.animation {
            Animation::Keyframed(keys) => Some(keys),
            _ => None,
        })
        .flatten()
        .map(|k| k.time.to_f64())
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    let gone = times.iter().rev().filter(|t| **t <= lt);
    let ahead = times.iter().filter(|t| **t > lt);
    gone.into_iter()
        .find_map(|t| heading(t - STEP, *t))
        .or_else(|| ahead.into_iter().find_map(|t| heading(*t, t + STEP)))
}

/// The turn Along path makes a flat layer at `lt`, in degrees. Nought with any
/// other Orient, and on a layer that never moves.
#[must_use]
pub fn flat_turn(tr: &TransformGroup, lt: f64) -> f64 {
    if tr.orient != Orient::AlongPath {
        return 0.0;
    }
    direction(tr, lt, true).map_or(0.0, |d| d.1.atan2(d.0).to_degrees())
}

/// The Orientation that lays a 3D layer's x axis along `d`, keeping it upright
/// as far as it can. Moving in the flat this is the 2D turn and nothing else.
fn along(d: (f64, f64, f64)) -> (f64, f64, f64) {
    let side = if d.0 < 0.0 { -1.0 } else { 1.0 };
    let reach = (d.0 * d.0 + d.2 * d.2).sqrt();
    (
        0.0,
        (-d.2 * side).atan2(d.0.abs()).to_degrees(),
        d.1.atan2(side * reach).to_degrees(),
    )
}

/// Whether `layer` is a camera or a light, which look where they point rather
/// than lie along it.
fn is_eye(layer: &Layer) -> bool {
    matches!(
        layer.kind,
        LayerKind::Camera { .. } | LayerKind::Light { .. }
    )
}

/// `layer`'s own placement at layer time `lt` with Along path in it, each
/// property read by `read`.
#[must_use]
pub fn placed_at(layer: &Layer, lt: f64, flat: bool, read: &dyn Fn(&Property) -> f64) -> Placement {
    let tr = &layer.transform;
    let mut p = tr.placement_at(lt, flat, read);
    if tr.orient != Orient::AlongPath {
        return p;
    }
    if flat {
        p.rotation_deg.2 += flat_turn(tr, lt);
    } else if let Some(d) = direction(tr, lt, false) {
        p.rotation_deg = if is_eye(layer) {
            let ahead = (p.position.0 + d.0, p.position.1 + d.1, p.position.2 + d.2);
            camera::look_at(p.position, ahead, p.rotation_deg).unwrap_or(p.rotation_deg)
        } else {
            camera::oriented(along(d), p.rotation_deg, false)
        };
    }
    p
}

/// The point `layer` at `here` looks at to face the camera at comp time `t`,
/// in its parent's space, with the layer's z axis pointing away from the
/// camera as an unturned layer's does. `None` on any other Orient, on a 2D
/// layer, and on a layer the camera is parented to, which would chase itself.
fn facing(
    comp: &Composition,
    layer: &Layer,
    t: f64,
    here: (f64, f64, f64),
    read: &Read<'_>,
    solves: Solves<'_>,
) -> Option<(f64, f64, f64)> {
    if layer.transform.orient != Orient::TowardsCamera || !layer.switches.three_d {
        return None;
    }
    if let Some(eye) = comp.active_camera(t) {
        if eye.id == layer.id || crate::model::layer_parent_chain(comp, eye.id).contains(&layer.id)
        {
            return None;
        }
    }
    let pose = match solves {
        Some((doc, store)) => {
            crate::track::camera_pose_at(doc, comp, t, store).map(|linked| linked.pose)
        }
        None => comp.camera_pose(t),
    }
    .unwrap_or_else(|| camera::default_pose(f64::from(comp.width), f64::from(comp.height)));
    let eye = match crate::model::parent_space(comp, layer, t, read, solves) {
        Some(space) => space.invert()?.point(pose.position),
        None => pose.position,
    };
    Some((
        here.0 + (here.0 - eye.0),
        here.1 + (here.1 - eye.1),
        here.2 + (here.2 - eye.2),
    ))
}

/// Turn `p` to face the camera, for a layer set to Towards camera.
pub fn face_camera(
    comp: &Composition,
    layer: &Layer,
    t: f64,
    read: &Read<'_>,
    solves: Solves<'_>,
    p: &mut Placement,
) {
    if let Some(away) = facing(comp, layer, t, p.position, read, solves) {
        p.rotation_deg =
            camera::look_at(p.position, away, p.rotation_deg).unwrap_or(p.rotation_deg);
    }
}

/// `layer`'s placement at comp time `t`, all of Orient in it: what the
/// compositor and its parent walk place it with.
#[must_use]
pub fn placed_in(
    comp: &Composition,
    layer: &Layer,
    t: f64,
    read: &Read<'_>,
    solves: Solves<'_>,
) -> Placement {
    let mut p = crate::model::placement(layer, t, read);
    face_camera(comp, layer, t, read, solves, &mut p);
    p
}

/// What Orient adds at comp time `t`, for the Viewer to draw the box where the
/// picture has it: the degrees on a flat layer, the turn outside the
/// Orientation on a 3D one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Turn {
    Flat(f64),
    Rows(Mat3),
}

/// See [`Turn`]. `None` when there is no turn.
#[must_use]
pub fn turn_at(
    comp: &Composition,
    layer: &Layer,
    t: f64,
    read: &Read<'_>,
    solves: Solves<'_>,
) -> Option<Turn> {
    let tr = &layer.transform;
    let lt = crate::time::layer_time(t, layer.start_offset.0);
    let eye = is_eye(layer);
    let flat = !layer.switches.three_d && !eye;
    match tr.orient {
        Orient::Off => None,
        Orient::AlongPath if flat => Some(Turn::Flat(flat_turn(tr, lt))),
        Orient::AlongPath => {
            let d = direction(tr, lt, false)?;
            Some(Turn::Rows(if eye {
                let look = camera::look_at((0.0, 0.0, 0.0), d, (0.0, 0.0, 0.0))?;
                camera::rotation(look)
            } else {
                camera::rotation_ae(along(d))
            }))
        }
        Orient::TowardsCamera => {
            let here = crate::model::placement(layer, t, read).position;
            let away = facing(comp, layer, t, here, read, solves)?;
            let look = camera::look_at(here, away, (0.0, 0.0, 0.0))?;
            Some(Turn::Rows(camera::rotation(look)))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::anim::{Keyframe, SideInterp};
    use crate::time::Rational;

    fn ramp(from: f64, to: f64) -> Property {
        let key = |sec: i64, value: f64| Keyframe {
            time: Rational::new(sec, 1).unwrap(),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let mut p = Property::fixed(0.0);
        p.animation = Animation::Keyframed(vec![key(0, from), key(2, to)]);
        p
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} is not {b}");
    }

    /// The file a layer is written to is the same bytes with Orient off, and
    /// one from before it reads as off, so an old project draws as it did.
    #[test]
    fn off_is_what_every_old_file_was() {
        let tr = TransformGroup::default();
        let json = serde_json::to_string(&tr).unwrap();
        assert!(!json.contains("orient\""), "{json}");
        let read: TransformGroup = serde_json::from_str(&json).unwrap();
        assert_eq!(read.orient, Orient::Off);
        let mut keyed = tr.clone();
        keyed.position_x = ramp(0.0, 100.0);
        keyed.position_y = ramp(0.0, 100.0);
        let p = keyed.placement_at(1.0, true, &|p| p.value_at(1.0));
        assert_eq!(p.rotation_deg, (0.0, 0.0, 0.0));
    }

    /// The tangent of a path, with the layer's own Rotation on top, at the
    /// frame asked for, and parked ends pointing the way the layer went.
    #[test]
    fn a_flat_layer_turns_along_its_path() {
        let mut tr = TransformGroup {
            orient: Orient::AlongPath,
            rotation: Property::fixed(10.0),
            position_x: ramp(0.0, 100.0),
            position_y: ramp(0.0, 100.0),
            ..TransformGroup::default()
        };
        close(flat_turn(&tr, 1.0), 45.0);
        close(flat_turn(&tr, 5.0), 45.0);
        close(flat_turn(&tr, -5.0), 45.0);
        tr.position_x = ramp(100.0, 0.0);
        tr.position_y = Property::fixed(0.0);
        close(flat_turn(&tr, 1.0).abs(), 180.0);
    }

    /// Moving in the flat, a 3D layer turns as a 2D one does, and moving along
    /// z its x axis lies along z.
    #[test]
    fn a_3d_layer_lays_its_x_axis_along_its_path() {
        let turned = |d| camera::rotation(camera::oriented(along(d), (0.0, 0.0, 0.0), false));
        let x_of = |m: Mat3| (m[0][0], m[1][0], m[2][0]);
        for d in [
            (1.0, 0.0, 0.0),
            (-1.0, 0.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (0.0, 0.0, -1.0),
            (1.0, 1.0, 1.0),
            (-2.0, 1.0, 3.0),
            (-2.0, -1.0, -3.0),
        ] {
            let (a, b, c): (f64, f64, f64) = d;
            let len = (a * a + b * b + c * c).sqrt();
            let x = x_of(turned(d));
            close(x.0, d.0 / len);
            close(x.1, d.1 / len);
            close(x.2, d.2 / len);
        }
    }
}
