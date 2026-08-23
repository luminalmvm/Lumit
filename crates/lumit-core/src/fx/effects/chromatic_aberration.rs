//! Chromatic aberration (docs/08 §3.15): the always-radial sibling of RGB
//! split's linear tinted-tap fringe — R pulled outward, B pulled inward, G and
//! alpha unshifted, growing from the frame centre.

use crate::fx::{cpu, normalise_tint_columns, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Which kernel a chromatic aberration instance draws through, and the numbers
/// that kernel wants (docs/impl/effect-registry.md §2.4).
///
/// The same fork RGB split has — Wavelength is a quality tier running a
/// different kernel (K-144), not a dial — but with this effect's own shapes:
/// there is no angle (the offset is always radial) and no per-tap scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fringe {
    /// The three tinted radial taps at fractions −1 / 0 / +1.
    Classic {
        /// Peak channel offset in raster pixels, reached at the corner distance
        /// from the frame centre.
        amount_px: f32,
        /// The three taps' tints, normalised per channel (K-167).
        tints: [[f32; 3]; 3],
        /// 0..1.
        mix: f32,
    },
    /// The Wavelength tier (K-144): RGB split's spectral machinery run radially.
    Spectral {
        /// Peak spectral offset in raster pixels.
        amount_px: f32,
        /// Tap count, rounded from the Samples slider.
        samples: i32,
        /// The three picker colours driving the dispersion gradient (A1/K-163);
        /// **not** normalised — the gradient reads them as authored.
        tints: [[f32; 3]; 3],
        /// 0..1.
        mix: f32,
    },
}

/// Chromatic aberration's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "chromatic_aberration",
    label = "Chromatic aberration",
    version = 1,
    category = Distortion,
    cost = Cheap,
    // Amount is raw px@comp, not % diag, so a tight %-diag padding cannot be
    // declared statically across every comp resolution; full-frame is the safe
    // static bound.
    roi = FullFrame,
)]
pub struct ChromaticAberration {
    /// px@comp (§2.3): peak channel offset, reached at the corner distance from
    /// the frame centre. Open above (K-135). This effect has only the radial
    /// shape and one purpose, so "how many pixels of fringe" is the honest
    /// unit. Declared `Px`, so the resolve step scales it by the
    /// §2.3 preview factor and
    /// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::
    /// rescale_spatial) moves it again if the stack is reused at another size —
    /// exactly what the old arm and `rescale_px` did between them.
    #[slider(
        min = 0.0,
        max = 20.0,
        default = 4.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub amount: f32,

    /// The three channel colours (P2/K-143), scene-linear RGBA (alpha ignored):
    /// the reusable three-colour picker tints the three radial taps. Defaults
    /// red / green / blue reproduce the classic R-outward / B-inward / G-anchor
    /// split bit-for-bit.
    #[colour(label = "Colour 1", default = [1.0, 0.0, 0.0, 1.0])]
    pub channel_colour_1: [f32; 4],

    /// See [`channel_colour_1`](Self::channel_colour_1).
    #[colour(label = "Colour 2", default = [0.0, 1.0, 0.0, 1.0])]
    pub channel_colour_2: [f32; 4],

    /// See [`channel_colour_1`](Self::channel_colour_1).
    #[colour(label = "Colour 3", default = [0.0, 0.0, 1.0, 1.0])]
    pub channel_colour_3: [f32; 4],

    /// K-144 quality tier, reusing RGB split's own spectral machinery (K-090):
    /// off (and absent on older projects) = the three tinted radial taps; on =
    /// `samples` spectral taps for a smooth rainbow fringe.
    #[toggle(default = false)]
    pub wavelength: bool,

    /// Wavelength mode's tap count (K-144). Rounded and clamped to 3..=64;
    /// ignored when Wavelength is off.
    #[slider(min = 3.0, max = 64.0, default = 16.0, hard_min = 3.0, hard_max = 64.0)]
    pub samples: f32,

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

impl ChromaticAberration {
    /// Which kernel to run and what to hand it (docs/impl/effect-registry.md
    /// §2.4).
    ///
    /// `amount` arrives already scaled by the §2.3 preview factor, so this only
    /// floors it — the same `.max(0.0)` the old arm applied to the same product,
    /// before the mode fork exactly as it was. The tap count is rounded in `f64`
    /// because that is what the old arm did. Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self) -> Fringe {
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        let amount_px = self.amount.max(0.0);
        let tints = [
            rgb(self.channel_colour_1),
            rgb(self.channel_colour_2),
            rgb(self.channel_colour_3),
        ];
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        if self.wavelength {
            Fringe::Spectral {
                amount_px,
                samples: f64::from(self.samples).round() as i32,
                tints,
                mix,
            }
        } else {
            Fringe::Classic {
                amount_px,
                // Normalised per channel (K-167), like RGB split's classic
                // mode: only the misaligned fringes take the colours.
                tints: normalise_tint_columns(tints),
                mix,
            }
        }
    }
}

/// Chromatic aberration's behaviour.
pub struct ChromaticAberrationDef;

impl EffectDef for ChromaticAberrationDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ChromaticAberration as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        match ChromaticAberration::read(p).packed() {
            Fringe::Classic {
                amount_px,
                tints,
                mix,
            } => cpu::chromatic_aberration(rgba, w, h, amount_px, tints, mix),
            // The radial spectral split: angle is meaningless when every offset
            // grows from the centre, and the old arm passed 0.0.
            Fringe::Spectral {
                amount_px,
                samples,
                tints,
                mix,
            } => cpu::spectral_split(rgba, w, h, amount_px, 0.0, true, samples, tints, mix),
        }
    }
}
