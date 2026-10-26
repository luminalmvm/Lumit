//! Lens distort (docs/08 §3.42): barrel and pincushion, described by the
//! frame's field of view — AE's Optics Compensation (docs/11 seed table).
//!
//! **In plain terms.** A wide lens bends straight lines. This adds that bend, or
//! takes it away. Field of view is the number a lens actually has: at
//! Orientation Horizontal, 40° means "this frame's width sees 40° of the world",
//! and the wider you say it is, the harder the picture bows. Reverse runs the
//! same maths backwards, so a distort and a reversed distort at the same
//! settings give the picture back.

use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EdgesMode,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Lens distort's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "lens_distort",
    label = "Lens distort",
    version = 1,
    category = Distortion,
    // One tangent (or arc tangent) and one bilinear tap a pixel. §1.6 usually
    // forbids per-pixel trigonometry; §3.42 records why it cannot be lifted out
    // here and what the oracle does about it.
    cost = Moderate,
    // A strong bend pulls the corners in from well outside the frame.
    roi = FullFrame,
    premultiplied = true,
    // The matte scales the displacement, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the distortion per pixel, read where the pixel lands: white \
         bends at the full Field of view, grey less, black not at all",
    ),
)]
pub struct LensDistort {
    /// Degrees: the frame's rectilinear field of view, spanning the half-extent
    /// [`orientation`](Self::orientation) picks. 0 is the exact identity (the
    /// kernel short-circuits rather than dividing by a zero tangent); the
    /// default bends visibly, per §1.2.
    #[slider(
        label = "Field of view",
        min = 0.0,
        max = 160.0,
        default = 40.0,
        hard_min = 0.0,
        hard_max = 179.0,
        unit = Degrees
    )]
    pub fov: f32,

    /// Off adds the fisheye (barrel — straight lines bow outward); on removes it
    /// (pincushion). The two are exact inverses of one another, not a sign flip
    /// (§3.42).
    #[toggle(label = "Reverse", default = false)]
    pub reverse: bool,

    /// Which half-extent the field of view spans, and so how much bend a given
    /// angle produces on a wide frame. Horizontal is AE's default and the one a
    /// lens specification means.
    #[choice(
        label = "Orientation",
        options = ["Horizontal", "Vertical", "Diagonal"],
        default = 0
    )]
    pub orientation: u32,

    /// px@comp: the optical centre the bend is about (point parameters are
    /// pixels). The schema default is nominal 1080p centre;
    /// `instantiate_for_raster` centres a fresh instance on the actual comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// The reusable Edges control (P3): what a sample that lands outside
    /// the frame reads. Transparent by default — a bend that reaches past the
    /// border has genuinely nothing there, and repeating the border pixel into a
    /// fan reads as a fault.
    #[choice(
        label = "Edges",
        options = *crate::fx::builtins::EDGE_OPTIONS,
        default = 0
    )]
    pub edge: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl LensDistort {
    /// Below this the effect is the exact identity: the focal length would be a
    /// division by (very nearly) zero, and the bend would be invisible anyway.
    pub const MIN_FOV_DEG: f32 = 0.01;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// `half_kind` says which half-extent the focal length is built from; it
    /// stays a code rather than a length because the kernel already knows the
    /// raster and the host does not (the same reasoning Tile's per cents get).
    /// `tan_half_fov` is the one trig call that can be lifted out of the pixel
    /// loop, and it is (§1.6).
    #[must_use]
    pub fn packed(self) -> cpu::LensDistortParams {
        let fov = self.fov.clamp(0.0, 179.0);
        let active = fov >= Self::MIN_FOV_DEG;
        cpu::LensDistortParams {
            active,
            tan_half_fov: if active {
                (fov * 0.5).to_radians().tan()
            } else {
                0.0
            },
            reverse: self.reverse,
            half_kind: self.orientation.min(2),
            centre: [self.centre_x, self.centre_y],
            edge: EdgesMode::from_code(self.edge.min(2))
                .unwrap_or(EdgesMode::Transparent)
                .code(),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            frame: [0.0, 0.0],
        }
    }

    /// [`Self::packed`] for the op whose parameters are `p`, told the frame's
    /// size when a node graph box has set its picture down on a bigger raster.
    #[must_use]
    pub fn packed_in(p: Params<'_>) -> cpu::LensDistortParams {
        let mut packed = Self::read(p).packed();
        if let Some([_, _, w, h]) = crate::fx::schema::frame_rect(p) {
            packed.frame = [w, h];
        }
        packed
    }
}

/// Lens distort's behaviour.
pub struct LensDistortDef;

impl EffectDef for LensDistortDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LensDistort as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::lens_distort(rgba, w, h, &LensDistort::packed_in(p));
    }

    /// Taking the bend away pushes the picture outward, so it can reach well
    /// past where it was. Adding it pulls the picture in, and a matte or a Mix
    /// can leave it where it was, so the rectangle it was handed is always
    /// kept as well.
    fn bounds(&self, p: Params<'_>, rect: [f32; 4]) -> [f32; 4] {
        let l = LensDistort::packed_in(p);
        if !l.active || !l.reverse || l.frame == [0.0, 0.0] {
            return rect;
        }
        let [fw, fh] = l.frame;
        let half = match l.half_kind {
            1 => fh * 0.5,
            2 => (fw * fw + fh * fh).sqrt() * 0.5,
            _ => fw * 0.5,
        };
        let f = half / l.tan_half_fov.max(1e-6);
        // Where a point of the picture lands: the kernel reads it through
        // `atan`, so it lands where `tan` puts it.
        let lands = |x: f32, y: f32| {
            let (dx, dy) = (x - l.centre[0], y - l.centre[1]);
            let r = (dx * dx + dy * dy).sqrt();
            if r <= 0.0 {
                return (x, y);
            }
            let radius = f * (r / f).min(cpu::LENS_MAX_THETA).tan();
            (l.centre[0] + dx * radius / r, l.centre[1] + dy * radius / r)
        };
        // The map is radial and keeps its order, so the edge of the picture
        // lands on the edge of where it went. Walked in steps, from half a
        // pixel out for the bilinear tap, which is close enough with a pixel
        // of margin.
        const STEPS: usize = 32;
        let [x0, y0, x1, y1] = [rect[0] - 0.5, rect[1] - 0.5, rect[2] + 0.5, rect[3] + 0.5];
        let mut out = rect;
        for i in 0..=STEPS {
            let t = i as f32 / STEPS as f32;
            let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            for (px, py) in [lands(x, y0), lands(x, y1), lands(x0, y), lands(x1, y)] {
                out = [
                    out[0].min(px),
                    out[1].min(py),
                    out[2].max(px),
                    out[3].max(py),
                ];
            }
        }
        [out[0] - 1.0, out[1] - 1.0, out[2] + 1.0, out[3] + 1.0]
    }
}
