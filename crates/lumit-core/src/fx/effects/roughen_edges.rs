//! Roughen edges (docs/08 §3.57): the alpha edge chewed by a fractal — AE's
//! Roughen Edges.
//!
//! **In plain terms.** The shape's outline is eaten into by a cloud of noise, so
//! a clean cut-out stops looking cut out: a title gets a torn-paper edge, a key
//! gets grime, a logo gets burnt. Nothing inside the shape moves — this changes
//! only where the shape stops.
//!
//! It works in two passes, and the first is a trick worth knowing. To chew forty
//! pixels off an edge you need to know how far every pixel is *from* that edge,
//! which is normally a whole algorithm of its own. Blurring the picture by forty
//! pixels gives the same answer for nothing: the half-way contour of a blurred
//! alpha sits exactly where the original edge was, and its slope is forty pixels
//! wide. The second pass just re-cuts that soft alpha at a threshold the noise
//! wobbles. Drop shadow (§3.43) reuses the shipped blur for its own reasons;
//! this is the second time it has paid.
//!
//! The noise is [`crate::fx::noise`], the same module Fractal noise draws and
//! Turbulent displace steers by — one field, one seed, one WGSL twin.

use crate::fx::{
    cpu, noise, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup,
    Params,
};
use lumit_fx_macros::Effect;

/// The panel's one disclosure — the loop controls, as §3.38's are.
pub const ROUGHEN_EDGES_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "Evolution options",
    params: &["cycle_evolution", "cycle"],
    collapsed: true,
    visible_when: None,
    visible_when_lens_elements: None,
}];

/// A loop length means nothing until the field is looping, and an edge colour
/// means nothing until the edge is being coloured.
pub const ROUGHEN_EDGES_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "cycle",
        on: "cycle_evolution",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "edge_colour",
        on: "colour_edge",
        cond: EnabledCond::BoolIs(true),
    },
];

/// Roughen edges' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "roughen_edges",
    label = "Roughen edges",
    version = 1,
    category = Stylise,
    // A gaussian pass, then up to ten octaves of 3-D noise a pixel.
    cost = Moderate,
    // Border's own reach. Its hard maximum is open, so the padding is the
    // slider's 500 px@comp doubled.
    roi = PaddedPx(1000.0),
    premultiplied = true,
    seeded = true,
    groups = ROUGHEN_EDGES_GROUPS,
    enabled_when = ROUGHEN_EDGES_ENABLED_WHEN,
    // K-428: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Border per pixel: white chews the outline at the full Border,          grey takes finer bites, black leaves it alone",
    ),
)]
pub struct RoughenEdges {
    /// Which shape the chewing takes. Roughen is the signed field smoothed by
    /// Edge sharpness; Cut ignores the sharpness and leaves a hard bitten edge;
    /// Spiky folds the field about zero so its ridges become spikes.
    #[choice(
        label = "Edge type",
        options = ["Roughen", "Cut", "Spiky"],
        default = 0
    )]
    pub edge_type: u32,

    /// px@comp: how deep the chewing reaches, and — being the blur radius of the
    /// first pass — how wide the band it works in is. 0 is the exact identity.
    #[slider(
        min = 0.0,
        max = 500.0,
        default = 40.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub border: f32,

    /// Per cent: 100 leaves a hard edge, 0 a soft one that fades across the
    /// whole border.
    #[slider(
        label = "Edge sharpness",
        min = 0.0,
        max = 100.0,
        default = 70.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub edge_sharpness: f32,

    /// Per cent: how far the noise shifts the cut. 0 is **not** the identity —
    /// it re-cuts the outline at the half-way contour, which hardens a soft
    /// matte and is a useful thing to ask for (§3.57 decision 3).
    #[slider(
        label = "Fractal influence",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0
    )]
    pub fractal_influence: f32,

    /// px@comp: the size of one lump of the noise — §3.37's Scale under the
    /// name AE gives it here. Floored at a pixel so the reciprocal stays finite.
    #[slider(min = 1.0, max = 2000.0, default = 100.0, hard_min = 1.0, unit = Px)]
    pub scale: f32,

    /// px@comp: where the noise field's origin sits. Animate it to drift the
    /// chewing along the edge.
    #[slider(label = "Offset x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub offset_x: f32,

    /// px@comp; see [`offset_x`](Self::offset_x).
    #[slider(label = "Offset y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub offset_y: f32,

    /// How many octaves of lump are summed. AE's default is two, and two is
    /// what reads as torn paper; more reads as corrosion.
    #[counter(min = 1, max = 10, default = 2, hard_min = 1, hard_max = 10)]
    pub complexity: i32,

    /// Degrees: the field's depth coordinate. One full turn advances one cell,
    /// matching Fractal noise (§3.37 decision 3) and AE's revolutions.
    #[dial(default = 0.0, step = 45.0)]
    pub evolution: f32,

    /// On, Evolution loops seamlessly after [`cycle`](Self::cycle) turns.
    #[toggle(label = "Cycle evolution", default = false)]
    pub cycle_evolution: bool,

    /// Whole turns of Evolution before the field repeats. The loop is exact
    /// (§3.37 decision 4).
    #[counter(min = 1, max = 30, default = 1, hard_min = 1, hard_max = 30)]
    pub cycle: i32,

    /// On, the chewed band is painted in [`edge_colour`](Self::edge_colour) —
    /// AE's "… Color" edge types and its Photocopy, as a switch of their own
    /// (§3.57 decision 2).
    #[toggle(label = "Colour edge", default = false)]
    pub colour_edge: bool,

    /// Scene-linear RGBA; the alpha is ignored, the coverage supplying it.
    #[colour(label = "Edge colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub edge_colour: [f32; 4],

    /// Which field this instance chews with (§2.4).
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

impl RoughenEdges {
    /// Each octave's amplitude as a share of the last, and each octave's
    /// frequency as a multiple of the last: §3.38's textbook halving and
    /// doubling, fixed for the same reason — AE does not expose them here
    /// either, and an edge is judged by its shape.
    pub const GAIN: f32 = 0.5;
    /// See [`GAIN`](Self::GAIN).
    pub const LACUNARITY: f32 = 2.0;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    /// Every division and every fold happens here: the scale becomes a
    /// reciprocal, Evolution is folded into the cycle, the sharpness becomes the
    /// half-width of the cut, and the Edge type choice becomes two flags rather
    /// than a branch.
    #[must_use]
    pub fn packed(self) -> cpu::RoughenEdgesParams {
        let cycle = if self.cycle_evolution {
            self.cycle.clamp(1, 30)
        } else {
            0
        };
        // The subtract-the-floor form rather than `rem_euclid`, because that is
        // the form WGSL can spell op-for-op (§3.38's note).
        let turns = self.evolution / 360.0;
        let z = if cycle > 0 {
            let n = cycle as f32;
            turns - (turns / n).floor() * n
        } else {
            turns
        };
        cpu::RoughenEdgesParams {
            field: noise::FractalField {
                seed: self.seed,
                octaves: self.complexity.clamp(1, noise::MAX_OCTAVES as i32) as u32,
                gain: Self::GAIN,
                lacunarity: Self::LACUNARITY,
                perlin: true,
                // Spiky is the ridged sum; the other two are the signed one.
                turbulent: self.edge_type == 2,
                cycle,
            },
            inv_scale: 1.0 / self.scale.max(1e-3),
            offset: [self.offset_x, self.offset_y],
            z,
            border_px: self.border.max(0.0),
            // Border 0 is the exact identity by short-circuit: a zero-radius
            // blur followed by a re-threshold would harden the picture's own
            // antialiasing for an effect the user has turned off.
            active: self.border > 0.0,
            influence: (self.fractal_influence / 100.0).max(0.0),
            // Cut ignores the sharpness and takes the hardest cut there is.
            half_width: if self.edge_type == 1 {
                Self::MIN_HALF_WIDTH
            } else {
                (0.5 * (1.0 - self.edge_sharpness / 100.0)).max(Self::MIN_HALF_WIDTH)
            },
            colour: [
                self.edge_colour[0],
                self.edge_colour[1],
                self.edge_colour[2],
            ],
            colour_on: f32::from(u8::from(self.colour_edge)),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }

    /// The narrowest the cut is allowed to be. A hundredth of the alpha range,
    /// which across a blurred edge of any real Border is well under a pixel — so
    /// Cut is a hard cut that is still *antialiased*, rather than an aliased
    /// step. It is also what stops the smoothstep dividing by zero.
    pub const MIN_HALF_WIDTH: f32 = 1e-2;
}

/// Roughen edges' behaviour.
pub struct RoughenEdgesDef;

impl EffectDef for RoughenEdgesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RoughenEdges as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::roughen_edges(rgba, w, h, &RoughenEdges::read(p).packed());
    }
}
