//! Glow (docs/08 §3.3): exposure-aware bloom in scene-linear light. A
//! bright-pass with a soft knee, a gaussian on the leftover light (one, or a
//! stack of octaves under Exponential), and an additive recombine.

use crate::fx::{
    cpu, normalise_tint_columns, EffectDef, EffectMetadata, EffectSchema, ParamGroup, Params,
};
use lumit_fx_macros::Effect;

/// The halo's fringe, folded into one twirl (P4). Six rows for a control that
/// is off on a fresh instance would otherwise push Intensity and Tint off the
/// bottom of the panel, and the fringe is the same set of controls the
/// Chromatic aberration effect carries, so it reads as that effect tucked
/// inside this one.
pub const GLOW_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "Chromatic aberration",
    params: &[
        "chromatic",
        "chromatic_wavelength",
        "chromatic_samples",
        "chromatic_colour_1",
        "chromatic_colour_2",
        "chromatic_colour_3",
    ],
    collapsed: true,
    visible_when: None,
    visible_when_lens_elements: None,
}];

/// Glow's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "glow",
    label = "Glow",
    version = 1,
    category = Stylise,
    cost = Moderate,
    groups = GLOW_GROUPS,
    // Radius is raw px@comp, unbounded above, so a tight %-diag padding
    // cannot be declared statically across every comp resolution — full-frame is
    // the safe static bound (mirroring Chromatic aberration's own px@comp
    // parameter).
    roi = FullFrame,
    // The glow claims the injected Matte row inside its own maths — the matte
    // gates the bright pass, so it decides which pixels are *allowed to glow*,
    // not how much of a finished glow survives. The generic strength dissolve
    // does not also run.
    matte = (
        "matte",
        "gates which pixels may seed the halo, before the bright pass: light \
         only escapes from where the matte is bright, but it still spills \
         outward across dark matte — which fading the finished glow cannot do",
    ),
)]
pub struct Glow {
    /// Linear-light value above which pixels bloom. The one-sided hard range
    /// made concrete: clamped at zero below, unbounded above — HDR values
    /// beyond the slider are legal and glow harder (§2.1). Default 0.8 so
    /// highlights just shy of white already bloom on a fresh instance.
    #[slider(min = 0.0, max = 4.0, default = 0.8, hard_min = 0.0, unit = Raw)]
    pub threshold: f32,

    /// Soft-knee width: the threshold's onset is eased by a smoothstep over
    /// ±knee around it (§3.3 step 1), so the bloom fades in rather than snapping
    /// on. The id stays `knee` — a stable identifier, addressed by expressions
    /// and saved projects — while the panel reads "Softness".
    #[slider(
        label = "Softness",
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        hard_max = 1.0,
        unit = Raw
    )]
    pub knee: f32,

    /// px@comp (§2.3): the halo gaussian's half-width in real pixels,
    /// clamped at zero below and unbounded above, so a wide bloom is a matter of
    /// typing a larger number rather than hitting a cap. Declared `Px`, so the
    /// resolve step scales it by the preview factor and the generic rescale moves
    /// it again — what the old arm and `rescale_px` did between them.
    #[slider(
        min = 0.0,
        max = 200.0,
        default = 24.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// Sum the halo from [`OCTAVES`] gaussians instead of one, each half the
    /// width of the one above it and weighted by Falloff. Off (and absent on
    /// older projects) is the single gaussian, to the byte. The halo's shape is
    /// the picture, so it changes when it is asked to and not before.
    #[toggle(default = false)]
    pub exponential: bool,

    /// How steeply the octave stack falls away from the core: octave `i` weighs
    /// `2^(falloff·i)`, so a high value gathers the light into a bright core with
    /// a faint reach and a low one spreads it back out toward the plain
    /// gaussian. Ignored while Exponential is off.
    #[slider(
        min = 0.5,
        max = 4.0,
        default = 1.0,
        hard_min = 0.5,
        hard_max = 4.0,
        unit = Raw
    )]
    pub falloff: f32,

    /// Per cent of Radius: how far the finished halo's channels are pulled apart
    /// radially before it is added back, so a wide bloom breaks into colour
    /// toward the corners. The picture under it is untouched. 0 reads no taps.
    /// Labelled Amount because the twirl it sits in is the one that says
    /// chromatic aberration.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub chromatic: f32,

    /// The fringe's quality tier, the same fork Chromatic aberration and RGB
    /// split carry: off (and absent on older projects) is the three tinted
    /// radial taps, on is `chromatic_samples` spectral taps for a smooth rainbow
    /// rather than three coloured ghosts.
    #[toggle(label = "Wavelength", default = false)]
    pub chromatic_wavelength: bool,

    /// Wavelength's tap count, rounded and clamped to 3..=64. Ignored while
    /// Wavelength is off.
    #[slider(
        label = "Samples",
        min = 3.0,
        max = 64.0,
        default = 16.0,
        hard_min = 3.0,
        hard_max = 64.0,
        unit = Raw
    )]
    pub chromatic_samples: f32,

    /// The three taps' colours, at fractions −1 / 0 / +1 out from the frame
    /// centre. Defaults red, green and blue give the classic split, red pulled
    /// outward and blue inward. Classic normalises them per channel, so only the
    /// misaligned part takes the colour. Wavelength reads them as authored and
    /// runs the gradient between them.
    #[colour(label = "Colour 1", default = [1.0, 0.0, 0.0, 1.0])]
    pub chromatic_colour_1: [f32; 4],

    /// See [`chromatic_colour_1`](Self::chromatic_colour_1).
    #[colour(label = "Colour 2", default = [0.0, 1.0, 0.0, 1.0])]
    pub chromatic_colour_2: [f32; 4],

    /// See [`chromatic_colour_1`](Self::chromatic_colour_1).
    #[colour(label = "Colour 3", default = [0.0, 0.0, 1.0, 1.0])]
    pub chromatic_colour_3: [f32; 4],

    /// Gain on the added halo; 0 is the effect's neutral point (bit-exact
    /// passthrough, pinned by test).
    #[slider(min = 0.0, max = 10.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub intensity: f32,

    /// The halo's colour, scene-linear — HDR tints are legal.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub tint: [f32; 4],

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

/// How many gaussians Exponential stacks the halo from. Five reaches a
/// sixteenth of the Radius, which is a bright core on any halo wide enough to
/// want one, and it is a constant rather than a control because the octave
/// count is the shape's making and not a dial anyone should have to read about.
/// Fixed, so a half-resolution preview stacks what the full render will.
pub const OCTAVES: u32 = 5;

impl Glow {
    /// The halo's shape, threshold, knee, intensity, tint and mix, clamped
    /// exactly as the old resolve arm clamped them (docs/impl/effect-registry.md
    /// §2.4). The radius arrives already scaled by the §2.3 preview factor, so
    /// this only floors it, the same `.max(0.0)` the arm applied to the same
    /// product. The fringe is taken as a fraction of that same scaled radius, so
    /// it follows the halo it sits on into a preview. Both render
    /// paths read this one method, so the CPU reference and the WGSL kernel
    /// cannot drift apart.
    pub fn packed(self) -> (cpu::GlowHalo, f32, f32, f32, [f32; 4], f32) {
        let radius_px = self.radius.max(0.0);
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        let tints = [
            rgb(self.chromatic_colour_1),
            rgb(self.chromatic_colour_2),
            rgb(self.chromatic_colour_3),
        ];
        (
            cpu::GlowHalo {
                radius_px,
                octaves: if self.exponential { OCTAVES } else { 1 },
                falloff: self.falloff.clamp(0.5, 4.0),
                chromatic_px: radius_px * (self.chromatic / 100.0).clamp(0.0, 1.0),
                // Classic normalises per channel, so only the misaligned
                // fringes take the colours. Wavelength reads them as authored,
                // because the gradient runs between them. The same split
                // Chromatic aberration's own pack makes.
                fringe_tints: if self.chromatic_wavelength {
                    tints
                } else {
                    normalise_tint_columns(tints)
                },
                fringe_wavelength: self.chromatic_wavelength,
                // Rounded in f64, as every other spectral pack rounds it.
                fringe_samples: f64::from(self.chromatic_samples).round() as i32,
            },
            self.threshold.max(0.0),
            self.knee.clamp(0.0, 1.0),
            self.intensity.max(0.0),
            self.tint,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Glow's behaviour.
pub struct GlowDef;

impl EffectDef for GlowDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Glow as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (halo, threshold, knee, intensity, tint, mix) = Glow::read(p).packed();
        // No matte through the single-buffer dispatcher: it carries one
        // picture, and this effect's matte is a second one (the rule the depth
        // pass and the LUT already follow). The §1.6 oracle for the matted path
        // is `cpu::glow_shaped` called directly from the lumit-gpu test, which
        // can upload it.
        cpu::glow_shaped(
            rgba,
            w,
            h,
            &halo,
            threshold,
            knee,
            intensity,
            tint,
            mix,
            &[],
        );
    }
}
