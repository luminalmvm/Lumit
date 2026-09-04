//! Fractal noise (docs/08 §3.37): the seeded multi-octave generator that half of
//! AE-land is built from — clouds, smoke, turbulence maps, displacement fields,
//! wipe mattes, grunge.
//!
//! **In plain terms.** A repeatable pattern of soft blobs, with smaller blobs
//! laid over the big ones, and smaller ones over those. Complexity says how many
//! layers of that there are; Sub influence how much each layer counts for; Sub
//! scaling how much smaller each one is. Evolution is depth — turn the dial and
//! the pattern *moves through* itself rather than being redrawn, which is the
//! difference between smoke drifting and smoke flickering.
//!
//! The maths lives in [`crate::fx::noise`], not here, because the displacement
//! family drives its warp with exactly this field (docs/impl/ae-effect-parity.md).
//! One implementation, one WGSL twin, one oracle.

use crate::fx::{
    cpu, noise, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup,
    Params,
};
use lumit_fx_macros::Effect;

/// A twirl with no visibility condition — the shape all four of this effect's
/// groups take.
const fn group(
    label: &'static str,
    params: &'static [&'static str],
    collapsed: bool,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// The panel's four disclosures, in AE's own order so the two panels read alike.
pub const FRACTAL_NOISE_GROUPS: &[ParamGroup] = &[
    group(
        "Transform",
        &[
            "rotation",
            "uniform_scaling",
            "scale",
            "scale_width",
            "scale_height",
            "offset_x",
            "offset_y",
        ],
        false,
    ),
    group("Sub settings", &["sub_influence", "sub_scaling"], true),
    group("Evolution options", &["cycle_evolution", "cycle"], true),
];

/// Which rows go grey when the switch beside them takes over.
pub const FRACTAL_NOISE_ENABLED_WHEN: &[EnabledWhen] = &[
    // Uniform scaling decides which of the three size rows is in charge: one
    // number for both axes, or a width and a height.
    EnabledWhen {
        param: "scale",
        on: "uniform_scaling",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "scale_width",
        on: "uniform_scaling",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "scale_height",
        on: "uniform_scaling",
        cond: EnabledCond::BoolIs(false),
    },
    // A loop length means nothing until the field is looping.
    EnabledWhen {
        param: "cycle",
        on: "cycle_evolution",
        cond: EnabledCond::BoolIs(true),
    },
];

/// Fractal noise's controls.
///
/// Every size here is px@comp (§2.3), declared `Px`, so the resolve step converts
/// each to the raster in play and
/// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::rescale_spatial)
/// moves them together if the stack is reused at another size. That is the
/// deliberate divergence from AE's per-cent Scale (§3.37 decision 1): a per cent
/// of an unnamed base is exactly the "pixels of whatever buffer I was handed"
/// §2.3 forbids.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "fractal_noise",
    label = "Fractal noise",
    version = 1,
    category = Generate,
    // Up to ten octaves of 3-D noise a pixel — no neighbour taps, but far from
    // pointwise.
    cost = Moderate,
    roi = Exact,
    // The field replaces the frame outright, so nothing of the input's colour is
    // read and there is nothing to unpremultiply (§2.2).
    premultiplied = true,
    seeded = true,
    groups = FRACTAL_NOISE_GROUPS,
    enabled_when = FRACTAL_NOISE_ENABLED_WHEN,
)]
pub struct FractalNoise {
    /// Value interpolates the lattice values themselves — cheap, and slightly
    /// blocky. Perlin interpolates the lattice *slopes*, which is the shape
    /// everyone means by "clouds".
    #[choice(label = "Noise type", options = ["Value", "Perlin"], default = 1)]
    pub noise_type: u32,

    /// Basic sums the signed octaves — soft and cloud-like. Turbulent folds each
    /// octave about zero first — ridged and smoke-like, and what a displacement
    /// map usually wants, which is why it is the default (as it is in AE).
    #[choice(label = "Fractal type", options = ["Basic", "Turbulent"], default = 1)]
    pub fractal_type: u32,

    /// Flip the finished field, after contrast and brightness.
    ///
    /// Labelled "Invert noise" rather than AE's bare "Invert" because the matte
    /// pair puts an **Invert** row at the bottom of every effect's panel, and
    /// two rows a panel apart with the same word on them is a question nobody
    /// should have to answer twice. The stored id is still `invert`.
    #[toggle(label = "Invert noise", default = false)]
    pub invert: bool,

    /// Per cent about the mid-grey pivot: 0 flattens the field to grey, 100
    /// leaves it, above 100 drives it toward black and white.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub contrast: f32,

    /// Per cent added after Contrast; ±100 covers the whole range on its own.
    #[slider(min = -200.0, max = 200.0, default = 0.0, unit = Percent)]
    pub brightness: f32,

    /// Degrees: turns the noise field under the frame, not the frame.
    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    /// On, [`scale`](Self::scale) sizes both axes; off, Scale width and Scale
    /// height do, and the field can be stretched into streaks.
    #[toggle(label = "Uniform scaling", default = true)]
    pub uniform_scaling: bool,

    /// px@comp: the size of one noise cell. Floored at a pixel so the reciprocal
    /// stays finite.
    #[slider(min = 1.0, max = 2000.0, default = 200.0, hard_min = 1.0, unit = Px)]
    pub scale: f32,

    /// px@comp, used only while Uniform scaling is off.
    #[slider(
        label = "Scale width",
        min = 1.0,
        max = 2000.0,
        default = 200.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub scale_width: f32,

    /// px@comp, used only while Uniform scaling is off.
    #[slider(
        label = "Scale height",
        min = 1.0,
        max = 2000.0,
        default = 200.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub scale_height: f32,

    /// px@comp: where the field's origin sits. Animate it to drift the whole
    /// pattern across the frame.
    #[slider(label = "Offset x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub offset_x: f32,

    /// px@comp; see [`offset_x`](Self::offset_x).
    #[slider(label = "Offset y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub offset_y: f32,

    /// How many octaves are summed. Capped at
    /// [`noise::MAX_OCTAVES`](crate::fx::noise::MAX_OCTAVES) so a `moderate`
    /// effect stays moderate.
    #[counter(min = 1, max = 10, default = 6, hard_min = 1, hard_max = 10, unit = Raw)]
    pub complexity: i32,

    /// Per cent: each octave's amplitude as a share of the one before. 0 leaves
    /// the first octave alone; 100 makes every octave count equally, which is
    /// noise rather than a surface.
    #[slider(
        label = "Sub influence",
        min = 0.0,
        max = 100.0,
        default = 60.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub sub_influence: f32,

    /// Per cent: each octave's cell size as a share of the one before. 50 is the
    /// textbook doubling of frequency; AE's 55 is a shade gentler, and is the
    /// default here for the same reason.
    #[slider(
        label = "Sub scaling",
        min = 5.0,
        max = 100.0,
        default = 55.0,
        hard_min = 5.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub sub_scaling: f32,

    /// Degrees: the field's depth coordinate. One full turn advances one cell,
    /// matching AE's revolutions — so keyframing it moves *through* the noise
    /// rather than reseeding it.
    #[dial(default = 0.0, step = 45.0)]
    pub evolution: f32,

    /// On, Evolution loops seamlessly after [`cycle`](Self::cycle) turns.
    #[toggle(label = "Cycle evolution", default = false)]
    pub cycle_evolution: bool,

    /// Whole turns of Evolution before the field repeats. The loop is exact
    /// (§3.37 decision 4), so a `cycle`-long animation tiles end to end.
    #[counter(min = 1, max = 30, default = 1, hard_min = 1, hard_max = 30, unit = Raw)]
    pub cycle: i32,

    /// Which field this instance draws (§2.4).
    #[seed]
    pub seed: u32,

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

impl FractalNoise {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// Everything expensive or platform-sensitive happens here, once: the
    /// rotation becomes a cosine/sine pair (WGSL's trigonometry is not correctly
    /// rounded and carries no guarantee of agreeing with Rust's, §1.6), every
    /// size becomes a reciprocal, the two per-cent dials become a gain and a
    /// lacunarity, and Evolution is folded into the cycle so the kernel never
    /// sees an unbounded angle. The sizes and the offset arrive already scaled to
    /// the raster by their declared `Px` unit. Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    #[must_use]
    pub fn packed(self) -> cpu::FractalNoiseParams {
        let (sx, sy) = if self.uniform_scaling {
            (self.scale, self.scale)
        } else {
            (self.scale_width, self.scale_height)
        };
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
        let rad = self.rotation.to_radians();
        cpu::FractalNoiseParams {
            field: noise::FractalField {
                seed: self.seed,
                octaves: self.complexity.clamp(1, noise::MAX_OCTAVES as i32) as u32,
                gain: (self.sub_influence / 100.0).clamp(0.0, 1.0),
                lacunarity: 100.0 / self.sub_scaling.clamp(5.0, 100.0),
                perlin: self.noise_type == 1,
                turbulent: self.fractal_type == 1,
                cycle,
            },
            cos_sin: [rad.cos(), rad.sin()],
            offset: [self.offset_x, self.offset_y],
            inv_scale: [1.0 / sx.max(1e-3), 1.0 / sy.max(1e-3)],
            z,
            contrast: self.contrast.max(0.0) / 100.0,
            brightness: self.brightness / 100.0,
            invert: self.invert,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Fractal noise's behaviour.
pub struct FractalNoiseDef;

impl EffectDef for FractalNoiseDef {
    fn schema(&self) -> &'static EffectSchema {
        &<FractalNoise as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::fractal_noise(rgba, w, h, &FractalNoise::read(p).packed());
    }
}
