//! RGB split (docs/08 §3.6): three tinted taps displaced along one angle, or —
//! with Wavelength on — a smooth spectral dispersion along the same offset.

use crate::fx::{cpu, normalise_tint_columns, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Which kernel an RGB split instance draws through, and the numbers that kernel
/// wants (docs/impl/effect-registry.md §2.4).
///
/// **Why `packed` returns this rather than a tuple.** Wavelength is a *quality
/// tier* (K-090), not a dial: on, the effect runs a different kernel with a
/// different uniform — which is why it had two `Resolved` variants before the
/// migration. One enum keeps that fork in one place, so the CPU reference and
/// the GPU wrapper cannot disagree about which mode an instance is in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Split {
    /// The classic three-tap split: taps 0/1 along −offset, tap 2 along
    /// +offset, each scaled by its own factor and multiplied by its tint.
    Classic {
        /// Peak tap offset in raster pixels.
        amount_px: f32,
        /// Shift direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// Per-tap displacement scale (FX-9), `[t0, t1, t2]`.
        scale: [f32; 3],
        /// The three taps' tints, normalised per channel (K-167).
        tints: [[f32; 3]; 3],
        /// 0..1.
        mix: f32,
    },
    /// The Wavelength tier: `samples` spectral taps along the same offset,
    /// tinted by the picker sampled as a gradient. Never radial — RGB split is
    /// linear-only since T17; chromatic aberration owns the radial shape.
    Spectral {
        /// Peak spectral offset in raster pixels.
        amount_px: f32,
        /// Shift direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// Tap count, rounded from the Samples slider (clamped 3..=64 by the
        /// tap builder both paths share).
        samples: i32,
        /// The three picker colours driving the dispersion gradient (A1/K-163);
        /// **not** normalised — the gradient reads them as authored.
        tints: [[f32; 3]; 3],
        /// 0..1.
        mix: f32,
    },
}

/// RGB split's controls.
///
/// A saved project that carries no `amount` at all now reads the declared
/// default rather than dropping the effect, which is the registry's rule for
/// every parameter (K-258); the old arm's `?` made a missing Amount silently
/// remove the whole op.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "rgb_split",
    label = "RGB split",
    version = 1,
    category = Distortion,
    cost = Cheap,
    roi = PaddedPctDiag(25.0),
    // K-427: the matte scales the displacement, inside the kernel (the
    // owner's rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Amount per pixel: white splits the channels the full distance, \
         grey less, black not at all",
    ),
)]
pub struct RgbSplit {
    /// px@comp (§2.3); the impact-frame staple is a keyframed spike on this.
    /// Declared `Px`, so the resolve step scales it to the raster in play and
    /// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::
    /// rescale_spatial) moves it again if the stack is reused at another size.
    #[slider(
        min = 0.0,
        max = 200.0,
        default = 8.0,
        hard_min = 0.0,
        hard_max = 500.0,
        unit = Px
    )]
    pub amount: f32,

    /// Degrees, linear mode: the direction R shifts (B mirrors).
    #[slider(
        min = -180.0,
        max = 180.0,
        default = 0.0,
        hard_min = -3600.0,
        hard_max = 3600.0
    )]
    pub angle: f32,

    /// Per-tap displacement scales (FX-9), per cent: each tap shifts by Amount
    /// times its own scale. The defaults 100 / 0 / 100 %, paired with the
    /// red / green / blue tints below, reproduce the classic split bit-for-bit.
    /// Open both sides (K-135): a negative scale flips a tap's direction.
    /// Labelled Red / Green / Blue for the classic case; each really scales its
    /// like-numbered tint.
    #[slider(label = "Red", min = -200.0, max = 200.0, default = 100.0)]
    pub red_amount: f32,

    /// See [`red_amount`](Self::red_amount).
    #[slider(label = "Green", min = -200.0, max = 200.0, default = 0.0)]
    pub green_amount: f32,

    /// See [`red_amount`](Self::red_amount).
    #[slider(label = "Blue", min = -200.0, max = 200.0, default = 100.0)]
    pub blue_amount: f32,

    /// The three tap tints (T17), scene-linear RGBA (alpha ignored). Defaults
    /// red / green / blue reproduce the classic channel-separated split
    /// bit-for-bit; any other colours cross-tint the fringe. Named
    /// `channel_colour_1/2/3` so the picker widget groups them into one swatch
    /// row.
    #[colour(label = "Colour 1", default = [1.0, 0.0, 0.0, 1.0])]
    pub channel_colour_1: [f32; 4],

    /// See [`channel_colour_1`](Self::channel_colour_1).
    #[colour(label = "Colour 2", default = [0.0, 1.0, 0.0, 1.0])]
    pub channel_colour_2: [f32; 4],

    /// See [`channel_colour_1`](Self::channel_colour_1).
    #[colour(label = "Colour 3", default = [0.0, 0.0, 1.0, 1.0])]
    pub channel_colour_3: [f32; 4],

    /// K-090 quality tier: off = the classic three-tap split (byte-identical to
    /// before this toggle existed); on = a smooth dispersion. The per-tap scales
    /// apply to the classic mode only; the tints drive both.
    #[toggle(default = false)]
    pub wavelength: bool,

    /// Wavelength mode's tap count (FX-9/K-144): more taps fill the same
    /// ±offset span more densely. Rounded and clamped to 3..=64; ignored in the
    /// classic mode.
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

impl RgbSplit {
    /// The three taps' tints as scene-linear RGB, alpha dropped — the form both
    /// modes read, and the input the classic mode normalises.
    fn tints(self) -> [[f32; 3]; 3] {
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        [
            rgb(self.channel_colour_1),
            rgb(self.channel_colour_2),
            rgb(self.channel_colour_3),
        ]
    }

    /// Which kernel to run and what to hand it (docs/impl/effect-registry.md
    /// §2.4).
    ///
    /// `amount` arrives already converted from % diagonal by the resolve step,
    /// so this only floors it — the same `.max(0.0)` the old arm applied to the
    /// same product, and applied *before* the mode fork exactly as it was. The
    /// per-tap scales and the tap count are computed in `f64` because that is
    /// what the old arm's expressions did; the two agree to well within the §1.6
    /// tolerance, but a migration must not change a single arithmetic step.
    /// Both render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    pub fn packed(self) -> Split {
        let amount_px = self.amount.max(0.0);
        let angle_deg = self.angle;
        let tints = self.tints();
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        if self.wavelength {
            Split::Spectral {
                amount_px,
                angle_deg,
                samples: f64::from(self.samples).round() as i32,
                tints,
                mix,
            }
        } else {
            // Per cent → factor. Normalised per channel (K-167): aligned
            // regions pass through unchanged; the picker tints only the fringes.
            let scale = |v: f32| (f64::from(v) / 100.0) as f32;
            Split::Classic {
                amount_px,
                angle_deg,
                scale: [
                    scale(self.red_amount),
                    scale(self.green_amount),
                    scale(self.blue_amount),
                ],
                tints: normalise_tint_columns(tints),
                mix,
            }
        }
    }
}

/// RGB split's behaviour.
pub struct RgbSplitDef;

impl EffectDef for RgbSplitDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RgbSplit as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        match RgbSplit::read(p).packed() {
            Split::Classic {
                amount_px,
                angle_deg,
                scale,
                tints,
                mix,
            } => cpu::rgb_split(rgba, w, h, amount_px, angle_deg, scale, tints, mix),
            Split::Spectral {
                amount_px,
                angle_deg,
                samples,
                tints,
                mix,
            } => cpu::spectral_split(rgba, w, h, amount_px, angle_deg, false, samples, tints, mix),
        }
    }
}
