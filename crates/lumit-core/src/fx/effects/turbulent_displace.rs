//! Turbulent displace (docs/08 §3.38): the fractal-driven warp — the distort
//! family's anchor, and the first effect to *steer* with §3.37's noise core
//! rather than draw it.
//!
//! **In plain terms.** Every pixel asks a hidden pattern of swirls "which way,
//! and how far?", then fetches its colour from there instead of from where it
//! sits. Size is how big a swirl is, Amount how far it can pull, Complexity how
//! much fine detail rides on the big swirls. Evolution is depth: turn it and the
//! swirls churn through themselves rather than being redrawn, which is the
//! difference between water moving and water flickering.
//!
//! The pattern is [`crate::fx::noise`], the same module Fractal noise draws —
//! not a copy of it (docs/impl/ae-effect-parity.md). Point a Fractal noise and a
//! Turbulent displace at the same Seed, Size, Complexity and Evolution and the
//! swirls line up with the picture, which is the whole reason the core is a
//! module.

use crate::fx::{
    cpu, noise, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup,
    Params,
};
use lumit_fx_macros::Effect;

/// The panel's one disclosure — the loop controls, which most projects never
/// open. Everything that shapes the warp stays in the open.
pub const TURBULENT_DISPLACE_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "Evolution options",
    params: &["cycle_evolution", "cycle"],
    collapsed: true,
    visible_when: None,
    visible_when_lens_elements: None,
}];

/// A loop length means nothing until the field is looping.
pub const TURBULENT_DISPLACE_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "cycle",
    on: "cycle_evolution",
    cond: EnabledCond::BoolIs(true),
}];

/// Turbulent displace's controls.
///
/// Both lengths here are px@comp (§2.3), declared `Px`, so the resolve step
/// converts them to the raster in play — the same divergence from AE's per cent
/// that §3.37 decision 1 records for Fractal noise's Scale, for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "turbulent_displace",
    label = "Turbulent displace",
    version = 1,
    category = Distortion,
    // Up to ten octaves of 3-D noise, twice, plus a bilinear tap.
    cost = Moderate,
    // The Amount slider's own reach; its hard maximum is open, so the padding
    // is the slider's 500 px@comp doubled, and the pin ramp only ever shortens
    // the pull.
    roi = PaddedPx(1000.0),
    premultiplied = true,
    seeded = true,
    groups = TURBULENT_DISPLACE_GROUPS,
    enabled_when = TURBULENT_DISPLACE_ENABLED_WHEN,
    // K-395: the matte belongs inside the maths here — it scales the
    // displacement vector, which is the owner's own example of why §2.6 has an
    // override at all. The generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the displacement per pixel: white warps at the full Amount, grey \
         warps a little, black not at all — where a strength dissolve would blend \
         a fully-warped picture over an unwarped one and show both edges at once",
    ),
)]
pub struct TurbulentDisplace {
    /// Which components of the warp survive. Turbulent uses both, which swirls;
    /// Horizontal and Vertical keep one axis, which shears the picture into
    /// ripples along the other.
    #[choice(
        label = "Displacement",
        options = ["Turbulent", "Horizontal", "Vertical"],
        default = 0
    )]
    pub displacement: u32,

    /// px@comp: the farthest a pixel can be pulled. Signed — negative simply
    /// reads the field the other way round.
    #[slider(min = -500.0, max = 500.0, default = 50.0, unit = Px)]
    pub amount: f32,

    /// px@comp: the size of one swirl — Fractal noise's Scale under the name the
    /// warp wants. Floored at a pixel so the reciprocal stays finite.
    #[slider(min = 1.0, max = 2000.0, default = 100.0, hard_min = 1.0, unit = Px)]
    pub size: f32,

    /// How many octaves of swirl are summed. One is a smooth swell; three is
    /// where it reads as turbulence, which is what the effect is called.
    #[counter(min = 1, max = 10, default = 3, hard_min = 1, hard_max = 10)]
    pub complexity: i32,

    /// px@comp: where the noise field's origin sits. Animate it to drift the
    /// whole warp across the frame.
    #[slider(label = "Offset x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub offset_x: f32,

    /// px@comp; see [`offset_x`](Self::offset_x).
    #[slider(label = "Offset y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub offset_y: f32,

    /// Degrees: the field's depth coordinate. One full turn advances one cell,
    /// matching Fractal noise (§3.37 decision 3) and AE's revolutions.
    #[dial(default = 0.0, step = 45.0)]
    pub evolution: f32,

    /// On, Evolution loops seamlessly after [`cycle`](Self::cycle) turns.
    #[toggle(label = "Cycle evolution", default = false)]
    pub cycle_evolution: bool,

    /// Whole turns of Evolution before the field repeats. The loop is exact
    /// (§3.37 decision 4), so a `cycle`-long animation tiles end to end.
    #[counter(min = 1, max = 30, default = 1, hard_min = 1, hard_max = 30)]
    pub cycle: i32,

    /// Which of the frame's edges are held still. A pinned edge cannot move, so
    /// the displacement ramps to zero across the last `|Amount|` pixels before
    /// it — which is also what stops the warp reaching outside the frame there.
    #[choice(
        label = "Pinning",
        options = ["None", "All edges", "Left and right", "Top and bottom"],
        default = 1
    )]
    pub pinning: u32,

    /// Which field this instance warps by (§2.4).
    #[seed]
    pub seed: u32,

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

impl TurbulentDisplace {
    /// The salt that turns one seed into the second, decorrelated field
    /// (docs/08 §3.38 decision 3). Reading one field at two nearby points
    /// instead correlates the two components, and a correlated warp slides
    /// diagonally rather than swirling.
    pub const SEED_Y_SALT: u32 = 0x5bf0_3635;

    /// Each octave's amplitude as a share of the last, and each octave's
    /// frequency as a multiple of the last: the textbook halving and doubling.
    /// Fixed rather than exposed (§3.38 decision 2) — AE does not offer them on
    /// this effect either, and a warp is judged by its shape.
    pub const GAIN: f32 = 0.5;
    /// See [`GAIN`](Self::GAIN).
    pub const LACUNARITY: f32 = 2.0;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// Every division and every fold happens here, once: the size becomes a
    /// reciprocal, the pin band becomes a reciprocal, Evolution is folded into
    /// the cycle so the kernel never sees an unbounded angle, and the
    /// Displacement choice becomes a pair of axis multipliers rather than a
    /// branch. Both render paths read this one method, so the CPU reference and
    /// the WGSL kernel cannot drift apart.
    #[must_use]
    pub fn packed(self) -> cpu::TurbulentDisplaceParams {
        let cycle = if self.cycle_evolution {
            self.cycle.clamp(1, 30)
        } else {
            0
        };
        // The depth coordinate: turns, folded into the loop when there is one.
        // The subtract-the-floor form rather than `rem_euclid`, because that is
        // the form WGSL can spell op-for-op.
        let turns = self.evolution / 360.0;
        let z = if cycle > 0 {
            let n = cycle as f32;
            turns - (turns / n).floor() * n
        } else {
            turns
        };
        let axes = match self.displacement {
            1 => [1.0, 0.0],
            2 => [0.0, 1.0],
            _ => [1.0, 1.0],
        };
        let pin = match self.pinning {
            1 => [1.0, 1.0],
            2 => [1.0, 0.0],
            3 => [0.0, 1.0],
            _ => [0.0, 0.0],
        };
        cpu::TurbulentDisplaceParams {
            field: noise::FractalField {
                seed: self.seed,
                octaves: self.complexity.clamp(1, noise::MAX_OCTAVES as i32) as u32,
                gain: Self::GAIN,
                lacunarity: Self::LACUNARITY,
                // Perlin, and the folded sum: the two the warp wants, and
                // Fractal noise's own defaults, so the two effects line up.
                perlin: true,
                turbulent: true,
                cycle,
            },
            seed_y: self.seed ^ Self::SEED_Y_SALT,
            inv_size: 1.0 / self.size.max(1e-3),
            offset: [self.offset_x, self.offset_y],
            z,
            amount: self.amount,
            axes,
            pin,
            // The pin ramp is |Amount| wide, so a pinned edge cannot be reached
            // from outside the frame. Floored so a zero Amount does not divide.
            inv_pin_band: 1.0 / self.amount.abs().max(1e-3),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Turbulent displace's behaviour. `apply_cpu` runs the reference with no matte
/// — the single-buffer dispatcher carries no second picture, exactly as the
/// Gaussian blur's does.
pub struct TurbulentDisplaceDef;

impl EffectDef for TurbulentDisplaceDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TurbulentDisplace as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::turbulent_displace(rgba, w, h, &TurbulentDisplace::read(p).packed(), &[]);
    }
}
