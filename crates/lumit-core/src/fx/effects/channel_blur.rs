//! Channel blur (docs/08 §3.45): the separable gaussian of §3.8, four times
//! over, with a radius each for red, green, blue and alpha — AE's Channel Blur.
//!
//! **In plain terms.** An ordinary blur softens everything by the same amount.
//! This one gives each channel its own amount, which is how a picture is made to
//! look like a real lens recorded it (blue resolves worst on a sensor, so
//! softening blue alone reads as optics rather than as blur), and how an alpha
//! is feathered without touching the colour inside it.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Channel blur's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "channel_blur",
    label = "Channel blur",
    version = 1,
    category = BlurSharpen,
    cost = Moderate,
    // The largest radius any ONE channel can reach — the sliders' own hard
    // maximum in px@comp, exactly as the Gaussian blur declares its own (§3.8).
    roi = PaddedPx(2000.0),
    premultiplied = true,
    // The matte scales the amount, inside the kernel (the owner's rule for
    // mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales all four blur radii per pixel: white blurs each channel at \
         its full radius, grey narrower, black not at all",
    ),
)]
pub struct ChannelBlur {
    /// Red's kernel half-width, px@comp (§2.3), scaled to the raster in play so
    /// a half-res preview matches.
    #[slider(
        label = "Red blur",
        min = 0.0,
        max = 500.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
    pub red: f32,

    /// Green's kernel half-width; see [`red`](Self::red).
    #[slider(
        label = "Green blur",
        min = 0.0,
        max = 500.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
    pub green: f32,

    /// Blue's kernel half-width; see [`red`](Self::red).
    ///
    /// **The one channel with a non-zero default**, per docs/08 §1.2: a real
    /// sensor resolves blue worst, so softening blue alone is both this
    /// effect's commonest single use and instantly legible as "this did
    /// something" — and it is the cheapest honest default, since the other
    /// three channels are then untouched.
    #[slider(
        label = "Blue blur",
        min = 0.0,
        max = 500.0,
        default = 40.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
    pub blue: f32,

    /// Alpha's kernel half-width; see [`red`](Self::red). Feathering a matte
    /// without touching the colour inside it is what this row is for.
    #[slider(
        label = "Alpha blur",
        min = 0.0,
        max = 500.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
    pub alpha: f32,

    /// On holds the border pixel outward, so a bright edge does not darken as
    /// the gather runs off the frame; off lets the frame edge fall away into
    /// transparency. AE's control is a switch and so is this one — not the
    /// three-way Edges enum, which Radial blur uses because it genuinely has
    /// three cases (docs/08 §3.45, §3.22's precedent).
    #[toggle(label = "Repeat edge pixels", default = true)]
    pub repeat_edge_pixels: bool,

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

impl ChannelBlur {
    /// The four radii in raster pixels, the edge policy and the mix
    /// (docs/impl/effect-registry.md §2.4). The radii arrive already converted
    /// from % diagonal by the resolve step, so this only floors them. Both
    /// render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    #[must_use]
    pub fn packed(self) -> ([f32; 4], u32, f32) {
        (
            [
                self.red.max(0.0),
                self.green.max(0.0),
                self.blue.max(0.0),
                self.alpha.max(0.0),
            ],
            // The shared edge codes (docs/08 §3.8): 1 Repeat, 0 Transparent.
            u32::from(self.repeat_edge_pixels),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Channel blur's behaviour.
pub struct ChannelBlurDef;

impl EffectDef for ChannelBlurDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ChannelBlur as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (radii, edge, mix) = ChannelBlur::read(p).packed();
        cpu::channel_blur(rgba, w, h, radii, edge, mix);
    }
}
