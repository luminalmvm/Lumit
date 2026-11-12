//! Tests for the Viewer and the compositor sharing one camera.

use super::*;

/// The Viewer draws a 3D layer's outline through
/// `lumit_core::camera::projection`, and the compositor draws its pixels
/// through this matrix. They have to stay the same rows, or the outline
/// slides off the picture.
#[test]
fn the_viewer_projection_is_the_compositors_camera() {
    let pose = lumit_core::model::CameraPose {
        zoom: 1800.0,
        position: (400.0, -150.0, -2100.0),
        rotation_deg: (-12.0, 28.0, 7.0),
        dof: None,
    };
    let gpu = camera_mat(1920, 1080, pose).to_cols_array_2d();
    let rows = lumit_core::camera::projection(&pose, 1920.0, 1080.0);
    // The matrix's rows 0, 1 and 3 are x, y and the divide.
    for (row, gpu_row) in rows.iter().zip([0, 1, 3]) {
        for (col, want) in row.iter().enumerate() {
            let got = f64::from(gpu[col][gpu_row]);
            assert!(
                (got - want).abs() <= 1e-4 * want.abs().max(1.0),
                "row {gpu_row} col {col}: {got} against {want}"
            );
        }
    }
}
