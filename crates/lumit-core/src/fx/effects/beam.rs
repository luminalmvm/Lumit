//! Beam (docs/08 §3.73): a tapered shaft of light travelling between two points
//! — AE's Beam.
//!
//! **In plain terms.** Two points, and a shaft of light drawn between them. Time
//! says how far the beam's head has travelled, Length how far its tail trails
//! behind, and the two thicknesses taper it from a fat root to a thin tip. The
//! inside colour is the core, the outside colour the rim, and Softness is how
//! much of the beam's width the one becomes the other in.
//!
//! It draws nothing that moves by itself: Time is an ordinary control the
//! timeline keyframes, which is docs/08 §3.53's ruling and the reason a preview
//! and an export cannot disagree about where the beam is.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Beam's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "beam",
    label = "Beam",
    version = 1,
    category = Generate,
    cost = Cheap,
    roi = Exact,
    // The beam's colour is written at its own coverage, which is already the
    // premultiplied form of "this colour, this much of it" (§3.34's reasoning).
    premultiplied = true,
)]
pub struct Beam {
    /// px@comp (K-260: point parameters are PIXELS). The schema defaults draw a
    /// diagonal across a nominal 1080p frame.
    #[slider(label = "Start x", min = 0.0, max = 3840.0, default = 240.0, unit = Px)]
    pub start_x: f32,

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "Start y", min = 0.0, max = 2160.0, default = 840.0, unit = Px)]
    pub start_y: f32,

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "End x", min = 0.0, max = 3840.0, default = 1680.0, unit = Px)]
    pub end_x: f32,

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "End y", min = 0.0, max = 2160.0, default = 240.0, unit = Px)]
    pub end_y: f32,

    /// How much of the run between the two points the beam occupies, per cent.
    /// AE's control and AE's default.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub length: f32,

    /// Where the beam's **head** has got to, per cent of the run. Keyframe it to
    /// fire the beam; at the default 100 the whole shaft is drawn, which is AE's
    /// picture and is visible the moment the effect is added (§1.2).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub time: f32,

    /// The beam's width at its tail, px@comp (§2.3), where AE's is layer pixels.
    #[slider(
        label = "Start thickness",
        min = 0.0,
        max = 200.0,
        default = 14.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub start_thickness: f32,

    /// The beam's width at its head, px@comp.
    #[slider(
        label = "End thickness",
        min = 0.0,
        max = 200.0,
        default = 3.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub end_thickness: f32,

    /// The share of the half-width the two colours cross over in, per cent. At 0
    /// the beam is a flat slab of the inside colour and the outside colour has
    /// nothing to colour, which is AE's own degenerate; 30 shows both.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 30.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub softness: f32,

    /// The core's colour. Scene-linear and open above 1 (§2.1), because a beam
    /// is a light.
    #[colour(label = "Inside colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub inside_colour: [f32; 4],

    /// The rim's colour.
    #[colour(label = "Outside colour", default = [0.10, 0.35, 1.0, 1.0], max = 4.0)]
    pub outside_colour: [f32; 4],

    /// On (AE's default and Lumit's), the layer that arrived stays under the
    /// beam. Off, the beam is all there is — which is what a light on its own
    /// solid wants.
    #[toggle(label = "Composite on original", default = true)]
    pub composite_on_original: bool,

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

impl Beam {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// Every reciprocal is taken here, once, and floored: a zero-length axis and
    /// a zero-length drawn interval both degenerate rather than dividing by zero
    /// (docs/14 §4). `active` is the §3.73 short-circuit — with the head and the
    /// tail in the same place there is no beam, and Time 0 is the bit-exact
    /// identity because of it.
    #[must_use]
    pub fn packed(self) -> cpu::BeamParams {
        let axis = [self.end_x - self.start_x, self.end_y - self.start_y];
        let len2 = (axis[0] * axis[0] + axis[1] * axis[1]).max(1e-6);
        let u1 = (self.time / 100.0).clamp(0.0, 1.0);
        let u0 = (u1 - (self.length / 100.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
        cpu::BeamParams {
            start: [self.start_x, self.start_y],
            axis,
            inv_len2: 1.0 / len2,
            u0,
            u1,
            inv_span: 1.0 / (u1 - u0).max(1e-6),
            half0: self.start_thickness.max(0.0) * 0.5,
            half1: self.end_thickness.max(0.0) * 0.5,
            soft: (self.softness / 100.0).clamp(0.0, 1.0).max(1e-3),
            inside: [
                self.inside_colour[0],
                self.inside_colour[1],
                self.inside_colour[2],
            ],
            outside: [
                self.outside_colour[0],
                self.outside_colour[1],
                self.outside_colour[2],
            ],
            active: u1 > u0,
            composite: self.composite_on_original,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Beam's behaviour.
pub struct BeamDef;

impl EffectDef for BeamDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Beam as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::beam(rgba, w, h, &Beam::read(p).packed());
    }
}
