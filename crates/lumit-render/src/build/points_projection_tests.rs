//! The points projection: the restriction really is the camera.

use super::*;

/// One camera and one placement, and the full comp-space projection of a
/// point in the layer's own pixels — the compositor's own arithmetic,
/// written here as the thing the restriction is measured against.
fn seen_by_the_compositor(cam: glam::Mat4, place: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 2] {
    let a = (cam * glam::Mat4::from_cols_array_2d(&place)).to_cols_array_2d();
    let at = |r: usize| a[0][r] * p[0] + a[1][r] * p[1] + a[2][r] * p[2] + a[3][r];
    let w = at(3);
    [at(0) / w, at(1) / w]
}

/// **The claim**, stated as a test: a particle drawn at the projection's
/// answer, *on the layer's flat plane*, ends up exactly where the
/// compositor would have put the particle at its true depth.
///
/// That is the whole of what the third axis does, and it is the one thing a
/// hand-written 3×4 could get subtly wrong in a way no picture test would
/// name. Both sides read `lumit_gpu`'s own matrices, so what is being
/// checked is the restriction and its inverse, not a second camera.
#[test]
fn a_projected_particle_lands_where_the_camera_would_have_put_it() {
    let cam = lumit_gpu::composite::camera_matrix(
        1920.0,
        1080.0,
        900.0,
        (860.0, 600.0, -900.0),
        (8.0, -20.0, 5.0),
    );
    let place = lumit_gpu::composite::place_matrix(
        (960.0, 540.0),
        (400.0, 300.0),
        (120.0, 80.0),
        17.0,
        -150.0,
        25.0,
        -40.0,
    );
    // The restriction, built the way `points_projection` builds it.
    let a = (cam * glam::Mat4::from_cols_array_2d(&place)).to_cols_array_2d();
    let row = |r: usize| [a[0][r], a[1][r], a[2][r], a[3][r]];
    let (rx, ry, rw) = (row(0), row(1), row(3));
    let h = [
        [rx[0], rx[1], rx[3]],
        [ry[0], ry[1], ry[3]],
        [rw[0], rw[1], rw[3]],
    ];
    let inv = invert3(&h).expect("a placed layer is not singular");
    let mut m = [[0.0f32; 4]; 3];
    for (r, mr) in m.iter_mut().enumerate() {
        for (c, out) in mr.iter_mut().enumerate() {
            *out = inv[r][0] * rx[c] + inv[r][1] * ry[c] + inv[r][2] * rw[c];
        }
    }
    let proj = lumit_core::fx::points::Projection { m };

    for p in [
        [0.0f32, 0.0, 0.0],
        [640.0, 200.0, 0.0],
        [640.0, 200.0, 250.0],
        [100.0, 700.0, -300.0],
        [800.0, 800.0, 120.0],
    ] {
        let (flat, scale) = proj.apply(p);
        let drawn = seen_by_the_compositor(cam, place, [flat[0], flat[1], 0.0]);
        let truth = seen_by_the_compositor(cam, place, p);
        assert!(
            (drawn[0] - truth[0]).abs() < 5e-2 && (drawn[1] - truth[1]).abs() < 5e-2,
            "a particle at {p:?} is drawn at {drawn:?} and belongs at {truth:?}"
        );
        // The plane itself is untouched, and off it the scale is the
        // perspective divide the compositor applied.
        if p[2] == 0.0 {
            assert!((scale - 1.0).abs() < 1e-4, "the plane foreshortened");
            assert!((flat[0] - p[0]).abs() < 1e-2 && (flat[1] - p[1]).abs() < 1e-2);
        }
    }
}
