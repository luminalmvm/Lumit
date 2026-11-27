use lumit_core::model::{CameraPose, Composition};
use lumit_gpu::composite::concat_place;

type Place = [[f32; 4]; 4];

/// How many comp pixels one of the layer's own pixels covers at most, read
/// off its world placement: the longest of the two axes' lengths, whichever
/// way round the matrix is laid out, so a turned or skewed layer is never
/// drawn short.
fn shape_raster_scale(m: [[f32; 4]; 4]) -> f32 {
    let len = |a: f32, b: f32| a.hypot(b);
    len(m[0][0], m[1][0])
        .max(len(m[0][1], m[1][1]))
        .max(len(m[0][0], m[0][1]))
        .max(len(m[1][0], m[1][1]))
}

/// Where a comp's layers are finally seen: through the camera of the comp
/// they are drawn into, in its frame, inside the placement of any collapsed
/// precomps on the way.
#[derive(Clone, Copy)]
pub(super) struct Seen {
    /// The camera the 3D layers are drawn through.
    pub(super) camera: CameraPose,
    /// The frame that camera draws into, comp pixels.
    frame: (f64, f64),
    /// The collapsed precomps' placement around this comp's own. `None` for
    /// a comp drawn on its own.
    outer: Option<Place>,
    /// One of those precomps is 3D, so a flat layer is seen by the camera too.
    on_plane: bool,
}

impl Seen {
    /// A comp drawn on its own, through `camera`.
    pub(super) fn own(camera: CameraPose, comp: &Composition) -> Self {
        Seen {
            camera,
            frame: (f64::from(comp.width), f64::from(comp.height)),
            outer: None,
            on_plane: false,
        }
    }

    /// As seen from inside a collapsed precomp layer placed by `place`.
    pub(super) fn within(self, place: Place, three_d: bool) -> Self {
        Seen {
            outer: Some(self.outer.map_or(place, |o| concat_place(o, place))),
            on_plane: self.on_plane || three_d,
            ..self
        }
    }

    /// How many frame pixels one of a layer's own pixels covers at most, the
    /// camera left out: `world` places it in its own comp.
    pub(super) fn flat(&self, world: Place) -> f32 {
        shape_raster_scale(self.outer.map_or(world, |o| concat_place(o, world)))
    }

    /// Whether the camera sees a layer with this 3D switch: its own, or a
    /// collapsed 3D precomp's around it.
    pub(super) fn camera_sees(&self, three_d: bool) -> bool {
        three_d || self.on_plane
    }

    /// How many frame pixels one of a layer's own pixels covers at most:
    /// `world` places it in its own comp and `size` is its own size. A layer
    /// the camera sees counts the camera's magnification too, read only over
    /// the part of it in front of the eye and in frame. The camera never draws
    /// it smaller than its placement alone would, and its share goes up in
    /// steps of an eighth of a doubling, so a moving camera does not have the
    /// layer rasterised afresh on every frame.
    pub(super) fn scale(&self, world: Place, three_d: bool, size: (f64, f64)) -> f32 {
        let flat = self.flat(world);
        if !self.camera_sees(three_d) {
            return flat;
        }
        let world = self.outer.map_or(world, |o| concat_place(o, world));
        match self.through_camera(world, size) {
            // A hair under a step stays on it, so rounding never adds one.
            Some(k) if k > f64::from(flat) => {
                let step = (k.log2() * 8.0 - 1e-6).ceil() / 8.0;
                2f64.powf(step) as f32
            }
            _ => flat,
        }
    }

    /// The most frame pixels one of the layer's pixels covers through the
    /// camera, over the part of its `size` box in front of the eye and in
    /// frame. `None` when no part of it is.
    fn through_camera(&self, world: Place, (w, h): (f64, f64)) -> Option<f64> {
        let (fw, fh) = self.frame;
        let p = lumit_core::camera::projection(&self.camera, fw, fh);
        // Each projected coordinate is affine in the layer's own (u, v): its
        // value at the origin, then its change along u and along v.
        let row = |r: [f64; 4]| {
            let at = |c: [f32; 4]| (0..4).map(|i| r[i] * f64::from(c[i])).sum::<f64>();
            [at(world[3]), at(world[0]), at(world[1])]
        };
        let (x, y, z) = (row(p[0]), row(p[1]), row(p[2]));
        let less = |a: [f64; 3], k: f64, b: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|i| k * a[i] - b[i])
        };
        // In front of the eye, then inside the frame's four edges. The depth
        // over the zoom is kept above a sixty-fourth, as close as a layer is
        // ever measured.
        let sides = [
            [z[0] - 1.0 / 64.0, z[1], z[2]],
            x,
            less(z, fw, x),
            y,
            less(z, fh, y),
        ];
        let mut part = vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
        for side in sides {
            part = clip(&part, side);
        }
        let magnified = |&(u, v): &(f64, f64)| {
            let at = |a: [f64; 3]| a[0] + a[1] * u + a[2] * v;
            let (px, py, pw) = (at(x), at(y), at(z));
            let along = |k: usize| {
                let dx = (x[k] * pw - px * z[k]) / (pw * pw);
                let dy = (y[k] * pw - py * z[k]) / (pw * pw);
                dx.hypot(dy)
            };
            along(1).max(along(2))
        };
        part.iter().map(magnified).reduce(f64::max)
    }
}

/// The part of polygon `poly` where `side[0] + side[1] u + side[2] v` is not
/// below nought.
fn clip(poly: &[(f64, f64)], side: [f64; 3]) -> Vec<(f64, f64)> {
    let f = |(u, v): (f64, f64)| side[0] + side[1] * u + side[2] * v;
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (&a, &b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        let (fa, fb) = (f(a), f(b));
        if fa >= 0.0 {
            out.push(a);
        }
        if (fa >= 0.0) != (fb >= 0.0) {
            let t = fa / (fa - fb);
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3D layer half way to the default camera is shown twice its size, so
    /// it is drawn twice its size. Out of frame, the camera adds nothing.
    #[test]
    fn the_camera_magnifies_only_what_it_sees() {
        let camera = lumit_core::camera::default_pose(1920.0, 1080.0);
        let seen = Seen {
            camera,
            frame: (1920.0, 1080.0),
            outer: None,
            on_plane: false,
        };
        let half = (camera.zoom * -0.5) as f32;
        let at = |x: f32| {
            lumit_gpu::composite::place_matrix_3d(
                (x, 540.0, half),
                (50.0, 50.0, 0.0),
                (100.0, 100.0, 100.0),
                (0.0, 0.0, 0.0),
            )
        };
        let size = (100.0, 100.0);
        assert!((seen.scale(at(960.0), true, size) - 2.0).abs() < 1e-4);
        assert_eq!(seen.scale(at(960.0), false, size), 1.0);
        assert_eq!(seen.scale(at(9000.0), true, size), 1.0);
    }
}
