use std::sync::Arc;

use crate::model::{Composition, Document, ResolvedLight};

pub struct View {
    /// The camera's projection, or none where the comp has no camera and the
    /// lights are drawn where they stand.
    rows: Option<[[f64; 4]; 3]>,
}

impl View {
    #[must_use]
    pub fn at(doc: &Arc<Document>, comp: &Composition, t: f64) -> Self {
        let (w, h) = (f64::from(comp.width), f64::from(comp.height));
        let rows = crate::track::active_camera_pose(doc, comp, t)
            .map(|pose| crate::camera::projection(&pose, w, h));
        Self { rows }
    }

    /// A light's `(x, y, half width, half height)` on the picture, in raster
    /// pixels at `px_scale`, as the camera sees it. None for a light behind
    /// the camera.
    #[must_use]
    pub fn place(&self, light: &ResolvedLight, px_scale: f32) -> Option<[f32; 4]> {
        let (x, y, z) = (light.position.0, light.position.1, light.z);
        let (x, y, k) = match &self.rows {
            None => (x, y, 1.0),
            Some(m) => {
                let row = |r: [f64; 4]| r[0] * x + r[1] * y + r[2] * z + r[3];
                let depth = row(m[2]);
                (depth > 1e-6).then(|| (row(m[0]) / depth, row(m[1]) / depth, 1.0 / depth))?
            }
        };
        let (w, h) = (light.half_size.0 * k, light.half_size.1 * k);
        Some([x, y, w, h].map(|v| v as f32 * px_scale))
    }
}
