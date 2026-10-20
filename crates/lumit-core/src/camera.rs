//! The camera's arithmetic (docs/impl/camera.md): the rotation the compositor
//! draws with, the aim of a two-node camera, the fixed views, the lens
//! conversions the settings dialog shows, and the circle of confusion depth of
//! field blurs by.
//!
//! Everything here is plain `f64` on plain tuples, because four crates and one
//! Dart file have to agree on it and none of them should have to agree on a
//! matrix type as well. The rotation order is the compositor's, `Ry · Rx · Rz`,
//! with Orientation outside it in After Effects' own `Rx · Ry · Rz`.

use crate::model::CameraPose;

/// A 3×3 matrix, row-major: `m[row][col]`.
pub type Mat3 = [[f64; 3]; 3];

fn rot_x(deg: f64) -> Mat3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rot_y(deg: f64) -> Mat3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rot_z(deg: f64) -> Mat3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

/// `Ry(y) · Rx(x) · Rz(z)` for angles in degrees.
#[must_use]
pub fn rotation(deg: (f64, f64, f64)) -> Mat3 {
    mul(&mul(&rot_y(deg.1), &rot_x(deg.0)), &rot_z(deg.2))
}

/// The `(x, y, z)` angles in degrees of a matrix built as `Ry · Rx · Rz`.
///
/// `M[1][2] = -sin x`, `M[0][2] = sin y cos x`, `M[2][2] = cos y cos x`,
/// `M[1][0] = cos x sin z`, `M[1][1] = cos x cos z`. At `cos x = 0` the camera
/// looks straight along its own y axis, y and z name the same turn, and the
/// whole of it is spent on y: there `M[0][0] = cos y` and `M[2][0] = -sin y`.
#[must_use]
pub fn euler_yxz(m: &Mat3) -> (f64, f64, f64) {
    let sin_x = (-m[1][2]).clamp(-1.0, 1.0);
    let rx = sin_x.asin();
    let cos_x = (1.0 - sin_x * sin_x).max(0.0).sqrt();
    if cos_x < 1e-9 {
        return (rx.to_degrees(), (-m[2][0]).atan2(m[0][0]).to_degrees(), 0.0);
    }
    (
        rx.to_degrees(),
        m[0][2].atan2(m[2][2]).to_degrees(),
        m[1][0].atan2(m[1][1]).to_degrees(),
    )
}

/// `Rx(x) · Ry(y) · Rz(z)` for angles in degrees: After Effects' order, which
/// Orientation turns by.
#[must_use]
pub fn rotation_ae(deg: (f64, f64, f64)) -> Mat3 {
    mul(&mul(&rot_x(deg.0), &rot_y(deg.1)), &rot_z(deg.2))
}

/// The `(x, y, z)` angles in degrees of a matrix built as `Rx · Ry · Rz`.
///
/// `M[0][2] = sin y`, `M[1][2] = -sin x cos y`, `M[2][2] = cos x cos y`,
/// `M[0][1] = -cos y sin z`, `M[0][0] = cos y cos z`. Looking straight along
/// x, x and z name the same turn and the whole of it goes on x.
#[must_use]
pub fn euler_xyz(m: &Mat3) -> (f64, f64, f64) {
    let sin_y = m[0][2].clamp(-1.0, 1.0);
    let ry = sin_y.asin();
    let cos_y = (1.0 - sin_y * sin_y).max(0.0).sqrt();
    if cos_y < 1e-9 {
        return (m[2][1].atan2(m[1][1]).to_degrees(), ry.to_degrees(), 0.0);
    }
    (
        (-m[1][2]).atan2(m[2][2]).to_degrees(),
        ry.to_degrees(),
        (-m[0][1]).atan2(m[0][0]).to_degrees(),
    )
}

/// The turn a layer draws with: its Orientation, After Effects' angles, outside
/// its X, Y and Z Rotation, which turn in Lumit's own `Ry · Rx · Rz`, or in
/// After Effects' `Rx · Ry · Rz` on a layer brought from there (`ae_order`).
/// Written back as Lumit's three angles, the ones nearest the rotations. With
/// no Orientation and Lumit's order, which is every file from before them, the
/// rotations come back exactly as they are.
#[must_use]
pub fn oriented(
    orientation: (f64, f64, f64),
    rotation: (f64, f64, f64),
    ae_order: bool,
) -> (f64, f64, f64) {
    if orientation == (0.0, 0.0, 0.0) && !ae_order {
        return rotation;
    }
    let rows = if ae_order {
        rotation_ae(rotation)
    } else {
        self::rotation(rotation)
    };
    nearest_turn(euler_yxz(&mul(&rotation_ae(orientation), &rows)), rotation)
}

/// [`oriented`] backwards: the rotations that, with this Orientation outside
/// them, make the whole of `turn`, the ones nearest `near`.
#[must_use]
pub fn unoriented(
    orientation: (f64, f64, f64),
    turn: (f64, f64, f64),
    ae_order: bool,
    near: (f64, f64, f64),
) -> (f64, f64, f64) {
    if orientation == (0.0, 0.0, 0.0) && !ae_order {
        return turn;
    }
    let o = rotation_ae(orientation);
    let back = [0, 1, 2].map(|r| [0, 1, 2].map(|c| o[c][r]));
    let rows = mul(&back, &rotation(turn));
    if ae_order {
        nearest_xyz(euler_xyz(&rows), near)
    } else {
        nearest_turn(euler_yxz(&rows), near)
    }
}

/// X, Y and Z Rotation written in one order (`from_ae`) rewritten in the other
/// (`to_ae`) for the same turn, the way of writing it nearest `near`.
#[must_use]
pub fn reordered(
    rows: (f64, f64, f64),
    from_ae: bool,
    to_ae: bool,
    near: (f64, f64, f64),
) -> (f64, f64, f64) {
    if from_ae == to_ae {
        return rows;
    }
    let m = if from_ae {
        rotation_ae(rows)
    } else {
        rotation(rows)
    };
    if to_ae {
        nearest_xyz(euler_xyz(&m), near)
    } else {
        nearest_turn(euler_yxz(&m), near)
    }
}

/// Lumit's angles for a turn After Effects writes as Orientation and X, Y and
/// Z Rotation, all in degrees.
///
/// After Effects builds each of the two as `Rx · Ry · Rz` and puts Orientation
/// outside the rotations, where Lumit turns by `Ry · Rx · Rz`, so the angles
/// only match when X or Y is zero. Every turn can be written two ways in
/// Lumit's order, and the one nearer `near` comes back, wound on by whole turns,
/// so a run of keys read one after the other doesn't spin between them.
#[must_use]
pub fn from_after_effects(
    orientation: (f64, f64, f64),
    rotation: (f64, f64, f64),
    near: (f64, f64, f64),
) -> (f64, f64, f64) {
    nearest_turn(
        euler_yxz(&mul(&rotation_ae(orientation), &rotation_ae(rotation))),
        near,
    )
}

/// The Orientation that turns a layer's z axis from `from` to look at `to`, as
/// After Effects' `lookAt` gives it: no turn about z, and nought when the two
/// points are the same.
#[must_use]
pub fn look_at_orientation(from: (f64, f64, f64), to: (f64, f64, f64)) -> (f64, f64, f64) {
    let d = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
    let len = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
    if len < 1e-9 {
        return (0.0, 0.0, 0.0);
    }
    // Rx · Ry turns z to (sin y, -sin x cos y, cos x cos y).
    (
        (-d.1).atan2(d.2).to_degrees(),
        (d.0 / len).clamp(-1.0, 1.0).asin().to_degrees(),
        0.0,
    )
}

/// How far apart two Orientations are, in degrees: the one turn that takes
/// `a` to `b`.
#[must_use]
pub fn turn_between_deg(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let (qa, qb) = (quaternion(&rotation_ae(a)), quaternion(&rotation_ae(b)));
    let dot: f64 = qa.iter().zip(&qb).map(|(p, q)| p * q).sum();
    2.0 * dot.abs().min(1.0).acos().to_degrees()
}

/// A turn as a unit quaternion `(w, x, y, z)`.
fn quaternion(m: &Mat3) -> [f64; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            0.25 * s,
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            (m[2][1] - m[1][2]) / s,
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[0][2] - m[2][0]) / s,
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
        ]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[1][0] - m[0][1]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
        ]
    };
    let len = q.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-12);
    q.map(|v| v / len)
}

fn from_quaternion([w, x, y, z]: [f64; 4]) -> Mat3 {
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// The Orientation `u` of the way from `a` to `b`, as After Effects moves
/// between two Orientation keys: along the shortest turn, not axis by axis.
/// The angles come back written the way `a` is and wound nearest the plain
/// blend of the two, so they don't jump about between.
#[must_use]
pub fn orientation_between(a: (f64, f64, f64), b: (f64, f64, f64), u: f64) -> (f64, f64, f64) {
    let blend = (
        a.0 + (b.0 - a.0) * u,
        a.1 + (b.1 - a.1) * u,
        a.2 + (b.2 - a.2) * u,
    );
    let qa = quaternion(&rotation_ae(a));
    let mut qb = quaternion(&rotation_ae(b));
    let mut dot: f64 = qa.iter().zip(&qb).map(|(p, q)| p * q).sum();
    // The same turn two ways round: take the short one.
    if dot < 0.0 {
        qb = qb.map(|v| -v);
        dot = -dot;
    }
    let q = if dot > 1.0 - 1e-12 {
        std::array::from_fn(|i| qa[i] + (qb[i] - qa[i]) * u)
    } else {
        let angle = dot.min(1.0).acos();
        let (wa, wb) = (
            ((1.0 - u) * angle).sin() / angle.sin(),
            (u * angle).sin() / angle.sin(),
        );
        std::array::from_fn(|i| qa[i] * wa + qb[i] * wb)
    };
    let len = q.iter().map(|v: &f64| v * v).sum::<f64>().sqrt().max(1e-12);
    let m = from_quaternion(q.map(|v| v / len));
    // The way of writing the turn that `a` is written in, so the numbers
    // don't swap to the other way part way between the keys.
    let one = euler_xyz(&m);
    let one = if a.1.to_radians().cos() >= 0.0 {
        one
    } else {
        (one.0 + 180.0, 180.0 - one.1, one.2 + 180.0)
    };
    let wind = |v: f64, n: f64| v - ((v - n) / 360.0).round() * 360.0;
    (
        wind(one.0, blend.0),
        wind(one.1, blend.1),
        wind(one.2, blend.2),
    )
}

/// [`nearest_turn`] for After Effects' order, whose other way of writing a
/// turn is `(x + 180, 180 - y, z + 180)`.
fn nearest_xyz(one: (f64, f64, f64), near: (f64, f64, f64)) -> (f64, f64, f64) {
    let other = (one.0 + 180.0, 180.0 - one.1, one.2 + 180.0);
    let wind = |a: f64, n: f64| a - ((a - n) / 360.0).round() * 360.0;
    let wound = |v: (f64, f64, f64)| (wind(v.0, near.0), wind(v.1, near.1), wind(v.2, near.2));
    let gap = |v: (f64, f64, f64)| {
        (v.0 - near.0).powi(2) + (v.1 - near.1).powi(2) + (v.2 - near.2).powi(2)
    };
    let (one, other) = (wound(one), wound(other));
    if gap(other) < gap(one) {
        other
    } else {
        one
    }
}

/// `one`, or the other way of writing the same turn in Lumit's order,
/// whichever is nearer `near` once both are wound on by whole turns.
fn nearest_turn(one: (f64, f64, f64), near: (f64, f64, f64)) -> (f64, f64, f64) {
    let other = (180.0 - one.0, one.1 + 180.0, one.2 + 180.0);
    let wound = |v: (f64, f64, f64)| {
        let wind = |a: f64, n: f64| a - ((a - n) / 360.0).round() * 360.0;
        (wind(v.0, near.0), wind(v.1, near.1), wind(v.2, near.2))
    };
    let gap = |v: (f64, f64, f64)| {
        (v.0 - near.0).powi(2) + (v.1 - near.1).powi(2) + (v.2 - near.2).powi(2)
    };
    let (one, other) = (wound(one), wound(other));
    if gap(other) < gap(one) {
        other
    } else {
        one
    }
}

/// The camera's own axes for a rotation in degrees: the columns of
/// `Ry · Rx · Rz`, in the order right, up, forward.
#[must_use]
pub fn axes(deg: (f64, f64, f64)) -> [(f64, f64, f64); 3] {
    let m = rotation(deg);
    [
        (m[0][0], m[1][0], m[2][0]),
        (m[0][1], m[1][1], m[2][1]),
        (m[0][2], m[1][2], m[2][2]),
    ]
}

/// The forward axis alone.
#[must_use]
pub fn forward(deg: (f64, f64, f64)) -> (f64, f64, f64) {
    axes(deg)[2]
}

/// The rotation that looks from `eye` at `target`, with the roll that the
/// stored rows add on top, composed the way After Effects' Orientation sits on
/// a two-node camera's aim. `None` when the two points coincide: no direction,
/// so the stored rotation is the aim.
#[must_use]
pub fn look_at(
    eye: (f64, f64, f64),
    target: (f64, f64, f64),
    stored_deg: (f64, f64, f64),
) -> Option<(f64, f64, f64)> {
    let d = (target.0 - eye.0, target.1 - eye.1, target.2 - eye.2);
    let len = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
    if len < 1e-9 {
        return None;
    }
    let f = (d.0 / len, d.1 / len, d.2 / len);
    // forward = (sin y cos x, -sin x, cos y cos x)
    let look_x = (-f.1).clamp(-1.0, 1.0).asin().to_degrees();
    let look_y = f.0.atan2(f.2).to_degrees();
    let look = mul(&rot_y(look_y), &rot_x(look_x));
    let m = mul(&look, &rotation(stored_deg));
    Some(euler_yxz(&m))
}

/// The eye that puts `pivot` exactly `distance` in front of a camera turned by
/// `deg`: what an orbit writes on a one-node camera.
#[must_use]
pub fn eye_behind(pivot: (f64, f64, f64), deg: (f64, f64, f64), distance: f64) -> (f64, f64, f64) {
    let f = forward(deg);
    (
        pivot.0 - f.0 * distance,
        pivot.1 - f.1 * distance,
        pivot.2 - f.2 * distance,
    )
}

/// A camera's or a light's own values: the eye, the rotation rows and, on a
/// two-node camera, the point of interest. All of them are read in the parent's
/// space, the point of interest too, as After Effects reads them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eye {
    pub position: (f64, f64, f64),
    pub rotation_deg: (f64, f64, f64),
    pub point_of_interest: Option<(f64, f64, f64)>,
}

/// How a camera or a light is read in a parent's space, which has no turn
/// of its own to give it when the parent is stretched.
impl Place {
    /// A turn made in this space, as the comp sees it. A camera has no scale,
    /// so a stretched space is squared up, forward first: the camera still
    /// looks exactly where the stretched space says, and only its up is
    /// nudged square to that.
    #[must_use]
    pub fn turn(&self, deg: (f64, f64, f64)) -> (f64, f64, f64) {
        let m = mul(&self.m, &rotation(deg));
        let unit = |v: (f64, f64, f64)| {
            let len = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
            (len > 1e-12).then(|| (v.0 / len, v.1 / len, v.2 / len))
        };
        let dot = |a: (f64, f64, f64), b: (f64, f64, f64)| a.0 * b.0 + a.1 * b.1 + a.2 * b.2;
        let Some(f) = unit((m[0][2], m[1][2], m[2][2])) else {
            return deg;
        };
        let up = (m[0][1], m[1][1], m[2][1]);
        let along = dot(up, f);
        let Some(u) = unit((up.0 - f.0 * along, up.1 - f.1 * along, up.2 - f.2 * along)) else {
            return deg;
        };
        let r = (
            u.1 * f.2 - u.2 * f.1,
            u.2 * f.0 - u.0 * f.2,
            u.0 * f.1 - u.1 * f.0,
        );
        let squared = [[r.0, u.0, f.0], [r.1, u.1, f.1], [r.2, u.2, f.2]];
        nearest_turn(euler_yxz(&squared), deg)
    }

    /// A camera or light's own values carried through this space: where it
    /// is in the comp from its values in a parent's space, or, through an
    /// inverse, the other way. A two-node camera is aimed on both sides, and
    /// its rows stay what is added on top of the aim.
    #[must_use]
    pub fn carry(&self, eye: Eye) -> Eye {
        let position = self.point(eye.position);
        let Some(poi) = eye.point_of_interest else {
            return Eye {
                position,
                rotation_deg: self.turn(eye.rotation_deg),
                point_of_interest: None,
            };
        };
        let aimed = look_at(eye.position, poi, eye.rotation_deg).unwrap_or(eye.rotation_deg);
        let point_of_interest = self.point(poi);
        let total = self.turn(aimed);
        let rotation_deg = match look_at(position, point_of_interest, (0.0, 0.0, 0.0)) {
            Some(aim) => {
                let a = rotation(aim);
                let at = [0, 1, 2].map(|r| [0, 1, 2].map(|c| a[c][r]));
                nearest_turn(euler_yxz(&mul(&at, &rotation(total))), eye.rotation_deg)
            }
            None => total,
        };
        Eye {
            position,
            rotation_deg,
            point_of_interest: Some(point_of_interest),
        }
    }

    /// A camera pose seen in this space, as the comp sees it. After Effects
    /// works depth of field out in the camera's own units, so the space's
    /// scale along the view stretches the focus distance, and the Blur Level
    /// with it, since that sets how long the lens is. A camera under a doubled
    /// parent focuses twice as far and blurs a card at twice the distance as
    /// it would unparented.
    #[must_use]
    pub fn pose(&self, pose: CameraPose) -> CameraPose {
        let ahead = self.vector(forward(pose.rotation_deg));
        let along = (ahead.0 * ahead.0 + ahead.1 * ahead.1 + ahead.2 * ahead.2).sqrt();
        // Rounding off an unscaled parent shouldn't move the focus.
        let along = if (along - 1.0).abs() < 1e-9 {
            1.0
        } else {
            along
        };
        CameraPose {
            position: self.point(pose.position),
            rotation_deg: self.turn(pose.rotation_deg),
            dof: pose.dof.map(|dof| crate::model::CameraDof {
                focus_distance: dof.focus_distance * along,
                blur_level: dof.blur_level * along,
                ..dof
            }),
            ..pose
        }
    }
}

/// The After Effects focal lengths the settings dialog offers, millimetres.
pub const PRESETS_MM: [f64; 9] = [15.0, 20.0, 24.0, 28.0, 35.0, 50.0, 80.0, 135.0, 200.0];

/// The default sensor width, millimetres: 35 mm film's frame.
pub const FILM_MM: f64 = 36.0;

/// A fresh camera's zoom for a comp `width` wide: the 50 mm lens on 35 mm film.
#[must_use]
pub fn default_zoom(width: f64) -> f64 {
    zoom_for_focal(50.0, FILM_MM, width)
}

/// What a comp with no camera layer is seen through, as in After Effects: a
/// fresh camera's 50 mm zoom behind the comp centre, looking down +z, with no
/// depth of field. A 3D layer at z = 0 lands exactly where a 2D one would.
#[must_use]
pub fn default_pose(width: f64, height: f64) -> CameraPose {
    let zoom = default_zoom(width);
    CameraPose {
        zoom,
        position: (width * 0.5, height * 0.5, -zoom),
        rotation_deg: (0.0, 0.0, 0.0),
        dof: None,
    }
}

/// A fresh camera's aperture for its zoom: what reads f/5.6 in the dialog.
#[must_use]
pub fn default_aperture(zoom: f64) -> f64 {
    aperture_for_f_stop(5.6, zoom)
}

#[must_use]
pub fn focal_mm(zoom: f64, film_mm: f64, comp_w: f64) -> f64 {
    if comp_w <= 0.0 {
        return 0.0;
    }
    zoom * film_mm / comp_w
}

#[must_use]
pub fn zoom_for_focal(focal_mm: f64, film_mm: f64, comp_w: f64) -> f64 {
    if film_mm <= 0.0 {
        return 0.0;
    }
    focal_mm * comp_w / film_mm
}

#[must_use]
pub fn angle_deg(zoom: f64, comp_w: f64) -> f64 {
    if zoom <= 0.0 {
        return 0.0;
    }
    2.0 * (comp_w / (2.0 * zoom)).atan().to_degrees()
}

#[must_use]
pub fn zoom_for_angle(angle_deg: f64, comp_w: f64) -> f64 {
    let half = (angle_deg / 2.0).to_radians().tan();
    if half <= 0.0 {
        return 0.0;
    }
    comp_w / (2.0 * half)
}

/// The After Effects relation between its own three numbers: the ten is
/// theirs, not a lens maker's.
#[must_use]
pub fn f_stop(zoom: f64, aperture: f64) -> f64 {
    if aperture <= 0.0 {
        return 0.0;
    }
    zoom / (10.0 * aperture)
}

#[must_use]
pub fn aperture_for_f_stop(f_stop: f64, zoom: f64) -> f64 {
    if f_stop <= 0.0 {
        return 0.0;
    }
    zoom / (10.0 * f_stop)
}

/// The circle of confusion's diameter on screen, comp pixels, for a layer at
/// depth `d` in front of the eye. Zero for anything on the sharp plane, at or
/// behind the eye, or with nothing to focus on.
#[must_use]
pub fn coc_px(dof: &crate::model::CameraDof, zoom: f64, d: f64) -> f64 {
    if d <= 0.0 {
        return 0.0;
    }
    coc_per_zoom(dof, zoom) * zoom * (d - dof.focus_distance).abs() / d
}

/// [`coc_px`] over zoom times how far off focus a point is over its distance.
/// After Effects draws it through a thin lens whose focal length is a tenth of
/// the zoom, the same ten as [`f_stop`], and Blur Level scales that focal
/// length rather than the blur. Zero when focused nearer than the lens itself,
/// which After Effects was never checked at.
#[must_use]
pub fn coc_per_zoom(dof: &crate::model::CameraDof, zoom: f64) -> f64 {
    let lens = zoom * (dof.blur_level / 100.0).max(0.0) / 10.0;
    if dof.focus_distance <= lens || zoom <= 0.0 {
        return 0.0;
    }
    dof.aperture * lens / (zoom * (dof.focus_distance - lens))
}

/// The Viewer's fixed 3D views (docs/07 §2.2). Active camera is the absence of
/// one, so it is not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    /// The three-quarter view a custom view starts from.
    Custom,
}

/// Where a view looks from, for a comp of this size. The six fixed views sit
/// `100 × width` from the comp centre with the same zoom, which is orthographic
/// to the pixel; the custom start is a three-quarter view at twice a fresh
/// camera's zoom.
#[must_use]
pub fn view_pose(view: View, width: f64, height: f64) -> CameraPose {
    let centre = (width * 0.5, height * 0.5, 0.0);
    let (deg, distance) = match view {
        View::Front => ((0.0, 0.0, 0.0), 100.0 * width),
        View::Back => ((0.0, 180.0, 0.0), 100.0 * width),
        View::Left => ((0.0, 90.0, 0.0), 100.0 * width),
        View::Right => ((0.0, -90.0, 0.0), 100.0 * width),
        View::Top => ((-90.0, 0.0, 0.0), 100.0 * width),
        View::Bottom => ((90.0, 0.0, 0.0), 100.0 * width),
        View::Custom => ((-25.0, 35.0, 0.0), default_zoom(width) * 2.0),
    };
    CameraPose {
        zoom: distance,
        position: eye_behind(centre, deg, distance),
        rotation_deg: deg,
        dof: None,
    }
}

/// A round number of comp pixels near `v`: 1, 2 or 5 times a power of ten.
/// What the ground's squares are sized by.
#[must_use]
pub fn nice(v: f64) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return 100.0;
    }
    let ten = 10f64.powf(v.log10().floor());
    let m = v / ten;
    ten * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

/// Project a comp-space point through a pose the way the compositor does:
/// `x' = cx + a.x · zoom / a.z` with `a` the point in the camera's frame, the
/// eye at its origin. Returns the comp-pixel position and whether the point is
/// in front of the eye; a point behind it still gets a number, but not one
/// worth drawing.
#[must_use]
pub fn project(
    pose: &CameraPose,
    width: f64,
    height: f64,
    p: (f64, f64, f64),
) -> ((f64, f64), bool) {
    let (x, y, w) = seen(pose, width, height, p);
    ((x, y), w * pose.zoom > 1e-6)
}

/// A comp-space point in the camera's own frame. Rotations invert by
/// transposing, so this never fails.
pub(crate) fn eye_frame(pose: &CameraPose) -> Place {
    let m = rotation(pose.rotation_deg);
    let back = Place {
        m: [
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
        ],
        t: (0.0, 0.0, 0.0),
    };
    let t = back.vector(pose.position);
    Place {
        t: (-t.0, -t.1, -t.2),
        ..back
    }
}

/// The picture position, and how far ahead of the eye over the zoom.
fn seen(pose: &CameraPose, width: f64, height: f64, p: (f64, f64, f64)) -> (f64, f64, f64) {
    let a = eye_frame(pose).point(p);
    let w = a.2 / pose.zoom.max(1e-9);
    let w = if w.abs() < 1e-9 { 1e-9 } else { w };
    (width * 0.5 + a.0 / w, height * 0.5 + a.1 / w, w)
}

/// [`project`] with After Effects' third number from `toComp`:
/// `zoom · (1 − 1/z)`, with `z` how far ahead of the eye the point is along
/// the view. Nought one pixel ahead of the eye, nearing the zoom far off.
/// [`unproject`] takes it back.
#[must_use]
pub fn project_depth(
    pose: &CameraPose,
    width: f64,
    height: f64,
    p: (f64, f64, f64),
) -> (f64, f64, f64) {
    let (x, y, w) = seen(pose, width, height, p);
    (x, y, pose.zoom - 1.0 / w)
}

/// The comp-space point that [`project_depth`] puts at picture position
/// `(x, y)` with that third number.
#[must_use]
pub fn unproject(
    pose: &CameraPose,
    width: f64,
    height: f64,
    p: (f64, f64, f64),
) -> (f64, f64, f64) {
    // A third number of the zoom itself is infinitely far off.
    let k = pose.zoom - p.2;
    let w = 1.0 / if k.abs() < 1e-9 { 1e-9 } else { k };
    let ahead = w * pose.zoom.max(1e-9);
    let a = ((p.0 - width * 0.5) * w, (p.1 - height * 0.5) * w, ahead);
    let m = rotation(pose.rotation_deg);
    let at = |r: usize| m[r][0] * a.0 + m[r][1] * a.1 + m[r][2] * a.2;
    (
        pose.position.0 + at(0),
        pose.position.1 + at(1),
        pose.position.2 + at(2),
    )
}

/// A placement in 3D, `p ↦ m · p + t`: a layer's transform, or a whole parent
/// chain multiplied out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub m: Mat3,
    pub t: (f64, f64, f64),
}

impl Place {
    /// `outer` applied after this one, the order a parent wraps its child.
    #[must_use]
    pub fn within(&self, outer: &Place) -> Place {
        Place {
            m: mul(&outer.m, &self.m),
            t: outer.point(self.t),
        }
    }

    #[must_use]
    pub fn point(&self, p: (f64, f64, f64)) -> (f64, f64, f64) {
        let v = self.vector(p);
        (v.0 + self.t.0, v.1 + self.t.1, v.2 + self.t.2)
    }

    /// A direction carried through, which the position leaves alone.
    #[must_use]
    pub fn vector(&self, v: (f64, f64, f64)) -> (f64, f64, f64) {
        let m = &self.m;
        let at = |r: usize| m[r][0] * v.0 + m[r][1] * v.1 + m[r][2] * v.2;
        (at(0), at(1), at(2))
    }

    /// The placement that undoes this one. `None` under a scale of nought,
    /// which flattens everything onto a line or a point.
    #[must_use]
    pub fn invert(&self) -> Option<Place> {
        let m = &self.m;
        let cofactor = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let det = m[0][0] * cofactor(1, 2, 1, 2) - m[0][1] * cofactor(1, 2, 0, 2)
            + m[0][2] * cofactor(1, 2, 0, 1);
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let inv = [
            [
                cofactor(1, 2, 1, 2) / det,
                -cofactor(0, 2, 1, 2) / det,
                cofactor(0, 1, 1, 2) / det,
            ],
            [
                -cofactor(1, 2, 0, 2) / det,
                cofactor(0, 2, 0, 2) / det,
                -cofactor(0, 1, 0, 2) / det,
            ],
            [
                cofactor(1, 2, 0, 1) / det,
                -cofactor(0, 2, 0, 1) / det,
                cofactor(0, 1, 0, 1) / det,
            ],
        ];
        let back = Place {
            m: inv,
            t: (0.0, 0.0, 0.0),
        };
        let t = back.vector(self.t);
        Some(Place {
            m: inv,
            t: (-t.0, -t.1, -t.2),
        })
    }
}

/// A layer's transform as the compositor places it (`lumit_gpu::place_matrix_3d`):
/// `T(position) · Ry · Rx · Rz · S(scale / 100) · T(−anchor)`, with any
/// Orientation already in the three angles ([`oriented`]).
#[must_use]
pub fn place(
    position: (f64, f64, f64),
    anchor: (f64, f64, f64),
    scale: (f64, f64, f64),
    deg: (f64, f64, f64),
) -> Place {
    let r = rotation(deg);
    let s = [scale.0 / 100.0, scale.1 / 100.0, scale.2 / 100.0];
    let mut m = r;
    for row in &mut m {
        for (cell, s) in row.iter_mut().zip(s) {
            *cell *= s;
        }
    }
    let turned = Place {
        m,
        t: (0.0, 0.0, 0.0),
    }
    .vector(anchor);
    Place {
        m,
        t: (
            position.0 - turned.0,
            position.1 - turned.1,
            position.2 - turned.2,
        ),
    }
}

/// [`project`] as three rows of a matrix, for taking many points through one
/// pose: `(x, y, z, 1)` goes to `(X, Y, W)` and the comp pixel is
/// `(X / W, Y / W)`. `W` is the depth over the zoom, so it is 1 on the plane
/// the camera sees 1:1 and above 0 for anything in front of the eye.
#[must_use]
pub fn projection(pose: &CameraPose, width: f64, height: f64) -> [[f64; 4]; 3] {
    let m = rotation(pose.rotation_deg);
    let e = pose.position;
    // One axis of the camera's own frame: `Rᵀ · (p - eye)`, as in `project`.
    let axis = |k: usize| {
        [
            m[0][k],
            m[1][k],
            m[2][k],
            -(m[0][k] * e.0 + m[1][k] * e.1 + m[2][k] * e.2),
        ]
    };
    let (ax, ay) = (axis(0), axis(1));
    let zoom = pose.zoom.max(1e-9);
    let w = axis(2).map(|v| v / zoom);
    [
        std::array::from_fn(|i| width * 0.5 * w[i] + ax[i]),
        std::array::from_fn(|i| height * 0.5 * w[i] + ay[i]),
        w,
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn close3(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
        close(a.0, b.0) && close(a.1, b.1) && close(a.2, b.2)
    }

    #[test]
    fn a_two_node_pose_points_at_its_point_of_interest() {
        let eye = (100.0, 200.0, -500.0);
        let poi = (400.0, -100.0, 300.0);
        let deg = look_at(eye, poi, (0.0, 0.0, 0.0)).unwrap();
        let f = forward(deg);
        let d = (poi.0 - eye.0, poi.1 - eye.1, poi.2 - eye.2);
        let len = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
        assert!(close3(f, (d.0 / len, d.1 / len, d.2 / len)));
        // The eye-behind inverse lands the eye back where it was.
        assert!(close3(eye_behind(poi, deg, len), eye));

        // Looking straight up or down, y and z are one turn, and the roll on
        // top still has to come out the right way round.
        for (poi, pitch) in [((0.0, -100.0, 0.0), 90.0), ((0.0, 100.0, 0.0), -90.0)] {
            let roll = (0.0, 0.0, 30.0);
            let deg = look_at((0.0, 0.0, 0.0), poi, roll).unwrap();
            let want = mul(&rot_x(pitch), &rotation(roll));
            let got = rotation(deg);
            let mut cells = got.iter().flatten().zip(want.iter().flatten());
            assert!(cells.all(|(a, b)| close(*a, *b)), "{deg:?}");
        }
    }

    /// A 200 by 100 card on its centre, Orientation X 40 outside a Y Rotation
    /// of 30: After Effects turns it by Rx(40) · Ry(30), which Lumit's own rows
    /// can't write as X 40 and Y 30. The corners are worked out by hand.
    #[test]
    fn orientation_turns_outside_the_rotations_as_after_effects_does() {
        use crate::anim::Property;
        let mut tr = crate::model::TransformGroup {
            anchor_x: Property::fixed(100.0),
            anchor_y: Property::fixed(50.0),
            position_x: Property::fixed(960.0),
            position_y: Property::fixed(540.0),
            rotation_y: Property::fixed(30.0),
            ..Default::default()
        };
        tr.orientation[0] = Property::fixed(40.0);
        let place = tr.placement_at(0.0, false, &|p| p.value_at(0.0)).place();
        let corners = [
            ((0.0, 0.0), (873.397_46, 469.558_40, 6.162_84)),
            ((200.0, 0.0), (1_046.602_54, 533.837_16, -70.441_60)),
            ((200.0, 100.0), (1_046.602_54, 610.441_60, -6.162_84)),
            ((0.0, 100.0), (873.397_46, 546.162_84, 70.441_60)),
        ];
        // The same turn on the rotations alone, X 40 and Y 30 in After
        // Effects' order, as an imported layer has them.
        let mut imported = tr.clone();
        imported.orientation[0] = Property::zero();
        imported.rotation_x = Property::fixed(40.0);
        imported.after_effects_order = true;
        let imported = imported
            .placement_at(0.0, false, &|p| p.value_at(0.0))
            .place();
        for ((x, y), want) in corners {
            for place in [place, imported] {
                let got = place.point((x, y, 0.0));
                let near = (got.0 - want.0).abs() + (got.1 - want.1).abs() + (got.2 - want.2).abs();
                assert!(near < 1e-3, "corner {x}, {y} at {got:?}");
            }
        }
        // Anchor Z pushes the card back along its own z, by Scale Z.
        tr.anchor_z = Property::fixed(20.0);
        tr.scale_z = Property::fixed(50.0);
        let pushed = tr.placement_at(0.0, false, &|p| p.value_at(0.0));
        let at = pushed.flat_position();
        let back = forward(pushed.rotation_deg);
        let want = (960.0 - back.0 * 10.0, 540.0 - back.1 * 10.0, -back.2 * 10.0);
        assert!(close3(at, want), "{at:?}");
        // Switching a layer's rotation order keeps its pose.
        let mut lumit = tr.clone();
        lumit.rotation_x = Property::fixed(25.0);
        lumit.rotation = Property::fixed(-40.0);
        let before = lumit.placement_at(0.0, false, &|p| p.value_at(0.0)).place();
        let mut switched = lumit.clone();
        for (prop, animation) in lumit.rotations_in_order(true).unwrap() {
            switched.get_mut(prop).unwrap().animation = animation;
        }
        switched.after_effects_order = true;
        let after = switched
            .placement_at(0.0, false, &|p| p.value_at(0.0))
            .place();
        for (x, y) in [(0.0, 0.0), (200.0, 100.0)] {
            assert!(close3(before.point((x, y, 0.0)), after.point((x, y, 0.0))));
        }
        // With no Orientation the rotations come back exactly as they were.
        assert_eq!(
            oriented((0.0, 0.0, 0.0), (10.0, 20.0, 30.0), false),
            (10.0, 20.0, 30.0)
        );
        // lookAt's Orientation points the layer's z from one point to the other.
        let m = rotation_ae(look_at_orientation(
            (10.0, 20.0, 30.0),
            (-290.0, 420.0, -170.0),
        ));
        let z = (m[0][2], m[1][2], m[2][2]);
        let d: (f64, f64, f64) = (-300.0 / 538.516_48, 400.0 / 538.516_48, -200.0 / 538.516_48);
        assert!(close3(z, d), "{z:?}");
    }

    /// After Effects keys an Orientation as one turn, so a key written on one
    /// angle gives the other two a key there, holding the turn as it was, and
    /// the picture only moves by the angle that was written.
    #[test]
    fn a_key_on_one_orientation_angle_keys_all_three() {
        use crate::anim::{Animation, Keyframe, Property, SideInterp};
        use crate::model::TransformProp as P;
        use crate::time::Rational;
        let key = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let keyed = |a: f64, b: f64| Property {
            animation: Animation::Keyframed(vec![key(0, a), key(2, b)]),
            extra: serde_json::Map::new(),
        };
        let mut tr = crate::model::TransformGroup {
            orientation: [keyed(0.0, 60.0), keyed(0.0, 80.0), keyed(0.0, 20.0)],
            ..Default::default()
        };
        let before = tr.orientation_at(1.0, &|p| p.value_at(1.0));
        let Animation::Keyframed(mut x) = tr.orientation[0].animation.clone() else {
            unreachable!()
        };
        x.insert(1, key(1, 45.0));
        let undo = tr.set_orientation_angle(P::OrientationX, Animation::Keyframed(x));
        assert_eq!(undo.len(), 3, "all three angles keyed");
        let after = tr.orientation_at(1.0, &|p| p.value_at(1.0));
        assert!(close3(after, (45.0, before.1, before.2)), "{after:?}");
        for (t, want) in [(0.0, (0.0, 0.0, 0.0)), (2.0, (60.0, 80.0, 20.0))] {
            assert!(close3(tr.orientation_at(t, &|p| p.value_at(t)), want));
        }
    }

    #[test]
    fn the_circle_of_confusion_is_zero_on_the_sharp_plane_and_grows_off_it() {
        let dof = crate::model::CameraDof {
            focus_distance: 1000.0,
            aperture: 20.0,
            blur_level: 100.0,
        };
        assert_eq!(coc_px(&dof, 1000.0, 1000.0), 0.0);
        assert!(coc_px(&dof, 1000.0, 2000.0) > 0.0);
        assert_eq!(coc_px(&dof, 1000.0, 0.0), 0.0);
        assert_eq!(coc_px(&dof, 1000.0, -5.0), 0.0);
        // After Effects 2026's own blur, read off white squares in a
        // 1280 x 720 comp at zoom 1200, aperture 400: the half-width of its box
        // blur at each focus, Blur Level and depth.
        for (focus, level, d, half_width) in [
            (1200.0, 100.0, 2200.0, 10.10),
            (1200.0, 100.0, 600.0, 22.23),
            (600.0, 100.0, 2200.0, 36.37),
            (2400.0, 100.0, 1200.0, 10.53),
            (3600.0, 100.0, 900.0, 20.68),
            (1200.0, 50.0, 4200.0, 7.51),
        ] {
            let dof = crate::model::CameraDof {
                focus_distance: focus,
                aperture: 400.0,
                blur_level: level,
            };
            let ours = coc_px(&dof, 1200.0, d) / 2.0;
            assert!(
                (ours / half_width - 1.0).abs() < 0.01,
                "focus {focus}, level {level}, depth {d}: {ours} against {half_width}"
            );
        }

        // After Effects' doubled rig: a camera turned -5 on Y, focused at 1500,
        // on a null turned -10 on X and 30 on Y and scaled 200. After Effects
        // keeps a card 3000 away sharp and blurs one 1500 away, as focus 3000
        // and Blur Level 200 do.
        let ae = |s: (f64, f64, f64)| {
            let turn = from_after_effects((0.0, 0.0, 0.0), (-10.0, 30.0, 0.0), (0.0, 0.0, 0.0));
            place((960.0, 540.0, 0.0), (0.0, 0.0, 0.0), s, turn)
        };
        let seen = |null: Place, rotation_deg| {
            let own = CameraPose {
                zoom: 1500.0,
                position: (100.0, -50.0, -1500.0),
                rotation_deg,
                dof: Some(crate::model::CameraDof {
                    focus_distance: 1500.0,
                    aperture: 150.0,
                    blur_level: 100.0,
                }),
            };
            let dof = null.pose(own).dof.unwrap();
            (dof.focus_distance, dof.aperture, dof.blur_level)
        };
        let (focus, aperture, level) = seen(ae((200.0, 200.0, 200.0)), (0.0, -5.0, 0.0));
        assert!(close(focus, 3000.0) && close(aperture, 150.0) && close(level, 200.0));
        let twice = crate::model::CameraDof {
            focus_distance: focus,
            aperture,
            blur_level: level,
        };
        let card = |d: f64| coc_px(&twice, 1500.0, d);
        assert!(card(3000.0) == 0.0 && card(1415.48) > 0.0);
        // Only the scale along the view counts: stretched on y, a camera
        // looking down the null's y focuses twice as far, one looking down
        // its z as far as it says.
        let stretched = ae((100.0, 200.0, 100.0));
        assert!(close(seen(stretched, (90.0, 0.0, 0.0)).0, 3000.0));
        assert!(close(seen(stretched, (0.0, -5.0, 0.0)).0, 1500.0));
    }

    #[test]
    fn the_front_view_is_the_default_camera_far_away_and_top_looks_down_y() {
        let front = view_pose(View::Front, 1920.0, 1080.0);
        assert!(close3(front.position, (960.0, 540.0, -192_000.0)));
        assert!(close3(front.rotation_deg, (0.0, 0.0, 0.0)));
        assert!(close(front.zoom, 192_000.0));
        let top = view_pose(View::Top, 1920.0, 1080.0);
        assert!(close3(forward(top.rotation_deg), (0.0, 1.0, 0.0)));
        assert!(close3(top.position, (960.0, 540.0 - 192_000.0, 0.0)));
        // Every fixed view looks at the comp centre.
        for view in [
            View::Back,
            View::Left,
            View::Right,
            View::Bottom,
            View::Custom,
        ] {
            let pose = view_pose(view, 1920.0, 1080.0);
            let f = forward(pose.rotation_deg);
            let at = (
                pose.position.0 + f.0 * pose.zoom,
                pose.position.1 + f.1 * pose.zoom,
                pose.position.2 + f.2 * pose.zoom,
            );
            assert!(close3(at, (960.0, 540.0, 0.0)), "{view:?} looks at {at:?}");
        }
    }

    #[test]
    fn projection_puts_the_default_camera_plane_one_to_one_and_halves_at_zoom() {
        let pose = CameraPose {
            zoom: 1000.0,
            position: (960.0, 540.0, -1000.0),
            rotation_deg: (0.0, 0.0, 0.0),
            dof: None,
        };
        let ((x, y), front) = project(&pose, 1920.0, 1080.0, (100.0, 200.0, 0.0));
        assert!(front);
        assert!(close(x, 100.0) && close(y, 200.0));
        let ((x, _), _) = project(&pose, 1920.0, 1080.0, (0.0, 540.0, 1000.0));
        assert!(close(x, 960.0 - 480.0));
        let (_, front) = project(&pose, 1920.0, 1080.0, (0.0, 0.0, -2000.0));
        assert!(!front);

        // The matrix form lands every point where `project` does.
        let turned = CameraPose {
            zoom: 1500.0,
            position: (300.0, -200.0, -900.0),
            rotation_deg: (20.0, -35.0, 10.0),
            dof: None,
        };
        let rows = projection(&turned, 1920.0, 1080.0);
        let p = (400.0, 600.0, 250.0);
        let at = |r: [f64; 4]| r[0] * p.0 + r[1] * p.1 + r[2] * p.2 + r[3];
        let ((x, y), _) = project(&turned, 1920.0, 1080.0, p);
        let w = at(rows[2]);
        assert!(w > 0.0 && close(at(rows[0]) / w, x) && close(at(rows[1]) / w, y));
    }
}
