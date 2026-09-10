//! The camera's arithmetic (docs/impl/camera.md): the rotation the compositor
//! draws with, the aim of a two-node camera, the fixed views, the lens
//! conversions the settings dialog shows, and the circle of confusion depth of
//! field blurs by.
//!
//! Everything here is plain `f64` on plain tuples, because four crates and one
//! Dart file have to agree on it and none of them should have to agree on a
//! matrix type as well. The rotation order is the compositor's, `Ry · Rx · Rz`.

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
/// whole of it is spent on y.
#[must_use]
pub fn euler_yxz(m: &Mat3) -> (f64, f64, f64) {
    let sin_x = (-m[1][2]).clamp(-1.0, 1.0);
    let rx = sin_x.asin();
    let cos_x = (1.0 - sin_x * sin_x).max(0.0).sqrt();
    if cos_x < 1e-9 {
        return (rx.to_degrees(), m[2][0].atan2(m[0][0]).to_degrees(), 0.0);
    }
    (
        rx.to_degrees(),
        m[0][2].atan2(m[2][2]).to_degrees(),
        m[1][0].atan2(m[1][1]).to_degrees(),
    )
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

/// The After Effects focal lengths the settings dialog offers, millimetres.
pub const PRESETS_MM: [f64; 9] = [15.0, 20.0, 24.0, 28.0, 35.0, 50.0, 80.0, 135.0, 200.0];

/// The default sensor width, millimetres: 35 mm film's frame.
pub const FILM_MM: f64 = 36.0;

/// A fresh camera's zoom for a comp `width` wide: the 50 mm lens on 35 mm film.
#[must_use]
pub fn default_zoom(width: f64) -> f64 {
    zoom_for_focal(50.0, FILM_MM, width)
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
    if d <= 0.0 || dof.focus_distance <= 0.0 || zoom <= 0.0 {
        return 0.0;
    }
    dof.aperture * (d - dof.focus_distance).abs() / d
        * (zoom / dof.focus_distance)
        * (dof.blur_level / 100.0).max(0.0)
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

/// Project a comp-space point through a pose the way the compositor does:
/// `x' = cx + a.x · zoom / (a.z + zoom)` with `a` the point in the camera's
/// frame, the eye at `a.z = -zoom`. Returns the comp-pixel position and
/// whether the point is in front of the eye; a point behind it still gets a
/// number, but not one worth drawing.
#[must_use]
pub fn project(
    pose: &CameraPose,
    width: f64,
    height: f64,
    p: (f64, f64, f64),
) -> ((f64, f64), bool) {
    let m = rotation(pose.rotation_deg);
    let d = (
        p.0 - pose.position.0,
        p.1 - pose.position.1,
        p.2 - pose.position.2,
    );
    // Rotation matrices invert by transposing: a = Rᵀ · d.
    let a = (
        m[0][0] * d.0 + m[1][0] * d.1 + m[2][0] * d.2,
        m[0][1] * d.0 + m[1][1] * d.1 + m[2][1] * d.2,
        m[0][2] * d.0 + m[1][2] * d.1 + m[2][2] * d.2,
    );
    // In the compositor's frame the eye is `zoom` behind the 1:1 plane, so a
    // point `a.z` in front of the eye is at `a.z - zoom` there.
    let w = a.2 / pose.zoom.max(1e-9);
    let in_front = a.2 > 1e-6;
    let w = if w.abs() < 1e-9 { 1e-9 } else { w };
    ((width * 0.5 + a.0 / w, height * 0.5 + a.1 / w), in_front)
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
    fn euler_round_trips_the_elementary_rotations_and_their_products() {
        for deg in [
            (30.0, 0.0, 0.0),
            (0.0, 40.0, 0.0),
            (0.0, 0.0, 50.0),
            (20.0, -35.0, 60.0),
            (-70.0, 120.0, -15.0),
        ] {
            let back = euler_yxz(&rotation(deg));
            assert!(close3(back, deg), "{deg:?} came back as {back:?}");
        }
    }

    #[test]
    fn an_unrotated_camera_looks_down_z_with_x_right_and_y_down() {
        let [right, up, fwd] = axes((0.0, 0.0, 0.0));
        assert!(close3(right, (1.0, 0.0, 0.0)));
        assert!(close3(up, (0.0, 1.0, 0.0)));
        assert!(close3(fwd, (0.0, 0.0, 1.0)));
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
    }

    #[test]
    fn a_two_node_pose_with_zero_rows_is_the_look_at_alone_and_rows_add_on_top() {
        let eye = (0.0, 0.0, -1000.0);
        let poi = (0.0, 0.0, 0.0);
        assert!(close3(
            look_at(eye, poi, (0.0, 0.0, 0.0)).unwrap(),
            (0.0, 0.0, 0.0)
        ));
        // Straight ahead plus a stored turn is that turn.
        let turned = look_at(eye, poi, (10.0, 20.0, 30.0)).unwrap();
        assert!(close3(turned, (10.0, 20.0, 30.0)));
        // Coincident points have no aim.
        assert!(look_at(poi, poi, (1.0, 2.0, 3.0)).is_none());
    }

    #[test]
    fn the_lens_formulas_invert_each_other() {
        let zoom = default_zoom(1920.0);
        assert!(close(zoom, 1920.0 * 50.0 / 36.0));
        assert!(close(focal_mm(zoom, FILM_MM, 1920.0), 50.0));
        assert!(close(zoom_for_angle(angle_deg(zoom, 1920.0), 1920.0), zoom));
        let ap = default_aperture(zoom);
        assert!(close(f_stop(zoom, ap), 5.6));
        assert!(close(aperture_for_f_stop(f_stop(zoom, ap), zoom), ap));
        assert_eq!(focal_mm(zoom, FILM_MM, 0.0), 0.0);
        assert_eq!(zoom_for_angle(0.0, 1920.0), 0.0);
    }

    #[test]
    fn the_circle_of_confusion_is_zero_on_the_sharp_plane_and_grows_off_it() {
        let dof = crate::model::CameraDof {
            focus_distance: 1000.0,
            aperture: 20.0,
            blur_level: 100.0,
        };
        assert_eq!(coc_px(&dof, 1000.0, 1000.0), 0.0);
        assert!(close(coc_px(&dof, 1000.0, 2000.0), 10.0));
        assert!(close(coc_px(&dof, 2000.0, 2000.0), 20.0));
        assert_eq!(coc_px(&dof, 1000.0, 0.0), 0.0);
        assert_eq!(coc_px(&dof, 1000.0, -5.0), 0.0);
        let half = crate::model::CameraDof {
            blur_level: 50.0,
            ..dof
        };
        assert!(close(coc_px(&half, 1000.0, 2000.0), 5.0));
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
    }
}
