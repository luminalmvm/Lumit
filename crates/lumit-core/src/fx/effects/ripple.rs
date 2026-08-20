//! Ripple (docs/08 §3.53): rings spreading from a point — AE's Ripple.
//!
//! **In plain terms.** A stone goes into the picture. Rings of distortion
//! spread out from where it landed, strongest a third of the way out and fading
//! to nothing at the rim, and turning Evolution sends the rings travelling
//! outward. Symmetric slides each pixel in and out along the radius, which reads
//! as a lens breathing; Asymmetric walks it round a small circle instead, which
//! is what water actually does and is why it is the default.
//!
//! **There is no speed control, and that is deliberate.** AE's Wave Speed reads
//! the clock, and an effect that reads the clock renders one picture in the
//! preview and another in the export (docs/08 §2.4). Evolution is the same
//! motion with the timeline holding the stopwatch.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Ripple's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "ripple",
    label = "Ripple",
    version = 1,
    category = Distortion,
    // One sine and cosine and one bilinear tap a pixel.
    cost = Cheap,
    // The rings can span the whole frame.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct Ripple {
    /// How far the rings reach, as a per cent of the comp diagonal (§2.3).
    /// Outside it the picture is untouched, exactly.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 30.0,
        hard_min = 0.0,
        unit = PctDiag
    )]
    pub radius: f32,

    /// px@comp: where the stone landed (K-260 — point parameters are pixels).
    /// The schema default is a nominal 1080p centre;
    /// [`instantiate_for_raster`](crate::fx::instantiate_for_raster) centres a
    /// fresh instance on the actual comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// Which way a pixel travels under the wave. Symmetric moves it along the
    /// radius only; Asymmetric adds the tangential half of the same wave, a
    /// quarter-turn out of phase, so the pixel walks a small circle.
    #[choice(label = "Type", options = ["Symmetric", "Asymmetric"], default = 1)]
    pub wave_type: u32,

    /// % diag: the farthest a pixel moves, at the envelope's peak (§3.53's
    /// first note — the `27⁄4` is why this number is literal).
    #[slider(
        label = "Wave height",
        min = 0.0,
        max = 10.0,
        default = 0.5,
        hard_min = 0.0,
        unit = PctDiag
    )]
    pub wave_height: f32,

    /// % diag: the distance between one crest and the next. Floored so the
    /// reciprocal stays finite.
    #[slider(
        label = "Wave width",
        min = 0.1,
        max = 20.0,
        default = 4.0,
        hard_min = 0.1,
        unit = PctDiag
    )]
    pub wave_width: f32,

    /// Degrees: one full turn sends one whole wave outward. AE's Wave Speed
    /// converts to a linear keyframe on this (§3.53 decision 2).
    #[dial(default = 0.0, step = 45.0)]
    pub evolution: f32,

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

impl Ripple {
    /// The reciprocal of the envelope's own peak, `ρ(1 − ρ)²` at `ρ = ⅓`, which
    /// is `4⁄27`. Multiplying by `27⁄4` is what makes Wave height literally the
    /// farthest a pixel moves rather than a number the envelope discounts
    /// (docs/08 §3.53 decision 1). A literal, not a computed quotient, so both
    /// paths multiply by the identical constant.
    pub const ENVELOPE_PEAK_RECIP: f32 = 6.75;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    /// Every division and every fold happens here: the radius and the wave
    /// width become reciprocals, Evolution becomes turns, and the Type choice
    /// becomes a flag.
    #[must_use]
    pub fn packed(self) -> cpu::RippleParams {
        let radius = self.radius.max(0.0);
        cpu::RippleParams {
            centre: [self.centre_x, self.centre_y],
            radius,
            // Floored so a zero radius does not divide; the kernel's
            // `r >= radius` test short-circuits before it is used anyway.
            inv_radius: 1.0 / radius.max(1e-3),
            amount: self.wave_height * Self::ENVELOPE_PEAK_RECIP,
            inv_width: 1.0 / self.wave_width.max(1e-3),
            turns: self.evolution / 360.0,
            asymmetric: self.wave_type == 1,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Ripple's behaviour.
pub struct RippleDef;

impl EffectDef for RippleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Ripple as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::ripple(rgba, w, h, &Ripple::read(p).packed());
    }
}
