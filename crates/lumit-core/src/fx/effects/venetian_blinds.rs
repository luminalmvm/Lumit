//! Venetian blinds (docs/08 §3.70): the frame closed by a rank of slats — AE's
//! Venetian Blinds.
//!
//! **In plain terms.** The picture is cut into stripes, and each stripe opens
//! from its own middle until nothing is left — a window blind being closed.
//! Completion says how far it has got, Direction which way the slats run, Width
//! how wide one slat is, and Feather how soft its edges are.
//!
//! It is [`super::linear_wipe`] with one line added: the distance across the
//! frame is folded into a single slat before it is thresholded, so one edge
//! becomes a rank of them.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Venetian blinds' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "venetian_blinds",
    label = "Venetian blinds",
    version = 1,
    category = Transition,
    cost = Trivial,
    roi = Exact,
    // The picture is scaled by a coverage, which is the premultiplied form of
    // "less of this pixel" (§3.46's reasoning).
    premultiplied = true,
    // The matte scales the amount, inside the kernel (the owner's rule for
    // mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Completion per pixel: the slats stand further open where the \
         matte is bright, so one part of the frame can be shut while \
         another is wide open",
    ),
)]
pub struct VenetianBlinds {
    /// How far the slats have closed, per cent. **50 by default, where AE's is
    /// 0**, for docs/08 §3.46's reason (§1.2: no no-op defaults).
    /// Closed 0..100: a wipe cannot be less than begun or more than
    /// complete, so the range is the parameter, and typing past either end
    /// would offer a picture that does not exist.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub completion: f32,

    /// Which way the slats run, degrees, **measured from straight up and turning
    /// clockwise** (§3.46's convention, and AE's). At 0° the slats are
    /// horizontal and the frame closes vertically.
    #[dial(default = 0.0, step = 15.0)]
    pub direction: f32,

    /// How wide one slat is, px@comp (§2.3), where AE's is raster pixels. The
    /// default of 20 is AE's own number, which at 1080p is AE's picture exactly.
    /// Floored at a pixel so the fold has a period.
    #[slider(min = 1.0, max = 500.0, default = 20.0, hard_min = 1.0, unit = Px)]
    pub width: f32,

    /// How soft each slat's edges are, px@comp. Keeps AE's 0, for §3.46's
    /// reason.
    #[slider(min = 0.0, max = 500.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub feather: f32,

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

impl VenetianBlinds {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// one trigonometric pair is taken here, host-side, for §1.6's reason;
    /// `(sin θ, −cos θ)` is "from straight up, clockwise" on a raster whose y
    /// grows downward. The slats' anchor is *not* here — they sit on the frame's
    /// own middle, which the kernel knows and the host does not (§3.46's
    /// precedent for the frame extent).
    #[must_use]
    pub fn packed(self) -> cpu::VenetianBlindsParams {
        let (sin, cos) = self.direction.to_radians().sin_cos();
        cpu::VenetianBlindsParams {
            normal: [sin, -cos],
            period: self.width.max(1.0),
            completion: (self.completion / 100.0).clamp(0.0, 1.0),
            // Floored so the hard-edged case is a step rather than a divide by
            // zero (docs/14 §4); neither path divides per pixel.
            band: self.feather.max(0.0).max(1e-3),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Venetian blinds' behaviour.
pub struct VenetianBlindsDef;

impl EffectDef for VenetianBlindsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<VenetianBlinds as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::venetian_blinds(rgba, w, h, &VenetianBlinds::read(p).packed());
    }
}
