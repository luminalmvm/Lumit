//! Twirl (docs/08 §3.51): the picture wrung round a point — AE's Twirl.
//!
//! **In plain terms.** Inside a circle the picture is turned, most at the middle
//! and not at all at the rim, so straight lines become spirals. Angle says how
//! hard, Radius how wide the circle is, Centre where it sits.
//!
//! The twist eases out toward the rim rather than stopping dead, which is why a
//! twirl blends into the untouched picture instead of leaving a visible ring.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Twirl's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "twirl",
    label = "Twirl",
    version = 1,
    category = Distortion,
    // One sine/cosine pair and one bilinear tap a pixel.
    cost = Cheap,
    // The circle can span the whole frame.
    roi = FullFrame,
    premultiplied = true,
    // K-427: the matte scales the displacement, inside the kernel (the
    // owner's rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Angle per pixel: white twirls the full Angle, grey less, black \
         not at all",
    ),
)]
pub struct Twirl {
    /// How far the middle is turned, degrees. A positive angle turns the picture
    /// clockwise on screen, because the raster's y grows downward — the reading
    /// every other angle in the catalogue has (§3.41).
    #[dial(default = 90.0, step = 15.0)]
    pub angle: f32,

    /// How wide the twirled circle is, px@comp (§2.3). Declared `Px`, so the
    /// resolve step scales it to the raster in play and a Half-resolution
    /// preview twirls the same part of the picture as the export.
    #[slider(
        min = 0.0,
        max = 2000.0,
        default = 650.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// px@comp: where the twirl's middle sits (K-260 — point parameters are
    /// pixels). The schema default is a nominal 1080p centre;
    /// [`instantiate_for_raster`](crate::fx::instantiate_for_raster) centres a
    /// fresh instance on the actual comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

impl Twirl {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// radius arrives as a reciprocal so the kernel runs no division; the angle
    /// stays an angle, because it is multiplied by a per-pixel falloff before
    /// any trigonometry is taken and so cannot be turned into a host-computed
    /// cosine/sine pair (§3.51's third note).
    #[must_use]
    pub fn packed(self) -> cpu::TwirlParams {
        let radius = self.radius.max(0.0);
        cpu::TwirlParams {
            centre: [self.centre_x, self.centre_y],
            radius,
            // Floored so a zero radius does not divide; the kernel's `r >=
            // radius` test short-circuits before the reciprocal is used anyway.
            inv_radius: 1.0 / radius.max(1e-3),
            angle: self.angle.to_radians(),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Twirl's behaviour.
pub struct TwirlDef;

impl EffectDef for TwirlDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Twirl as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::twirl(rgba, w, h, &Twirl::read(p).packed());
    }
}
