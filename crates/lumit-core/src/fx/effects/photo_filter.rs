//! Photo filter (docs/08 §3.61): a coloured glass held in front of the lens —
//! AE's Photo Filter.
//!
//! **In plain terms.** Before white balance was a menu, a photographer screwed a
//! coloured glass onto the lens to warm a shot up or cool it down. This is that
//! glass: the picture is multiplied by the filter's colour, Density says how
//! strong the glass is, and Preserve luminosity puts the exposure back
//! afterwards so the shot changes colour rather than getting darker.
//!
//! Twenty named filters ship — the six warming and cooling filters photographers
//! know by their Wratten numbers, eight plain colours, sepia, four deep ones and
//! an underwater — plus Custom, which takes any colour.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, Params};
use lumit_fx_macros::Effect;

/// A colour picker means nothing until the filter is the custom one.
pub const PHOTO_FILTER_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "colour",
    on: "filter",
    cond: EnabledCond::ChoiceIs(PhotoFilter::CUSTOM),
}];

/// Photo filter's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "photo_filter",
    label = "Photo filter",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: a multiply followed by a luma renormalisation does not commute with
    // premultiplied alpha.
    premultiplied = false,
    enabled_when = PHOTO_FILTER_ENABLED_WHEN,
    // The matte scales the amount, inside the kernel (the owner's rule for
    // mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Density per pixel: white holds the full Density of glass in \
         front of the lens, black none",
    ),
)]
pub struct PhotoFilter {
    /// Which glass. The twenty named ones are Lumit's own chromaticities under
    /// Adobe's names (§3.61); Custom takes [`colour`](Self::colour) instead.
    #[choice(
        options = [
            "Warming filter (85)",
            "Warming filter (LBA)",
            "Warming filter (81)",
            "Cooling filter (80)",
            "Cooling filter (LBB)",
            "Cooling filter (82)",
            "Red",
            "Orange",
            "Yellow",
            "Green",
            "Cyan",
            "Blue",
            "Violet",
            "Magenta",
            "Sepia",
            "Deep red",
            "Deep blue",
            "Deep emerald",
            "Deep yellow",
            "Underwater",
            "Custom",
        ],
        default = 0
    )]
    pub filter: u32,

    /// The glass's own colour, used while Filter is Custom. Scene-linear
    /// (§2.1); the alpha is ignored.
    #[colour(default = [0.85, 0.35, 0.05, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// Per cent: how much of the filter's colour is in front of the lens. 0 is
    /// the exact identity, 100 is the glass at full strength.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 25.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub density: f32,

    /// On, the pixel's Rec. 709 luma is restored after the multiply, so the
    /// picture changes colour and not exposure. Off, a strong filter really does
    /// stop light, which is AE's behaviour and a real glass's.
    #[toggle(label = "Preserve luminosity", default = true)]
    pub preserve_luminosity: bool,

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

impl PhotoFilter {
    /// The Custom option's index — the last, so adding a filter above it does
    /// not renumber a saved project.
    pub const CUSTOM: u32 = 20;

    /// The twenty named filters, as sRGB bytes. Stored in the form they are
    /// quoted in — a filter is named by its display colour — and decoded to
    /// scene-linear once, host-side, in [`packed`](Self::packed).
    pub const FILTERS: [[u8; 3]; 20] = [
        [0xEC, 0x8A, 0x00], // Warming filter (85)
        [0xFA, 0x96, 0x00], // Warming filter (LBA)
        [0xEB, 0xB1, 0x13], // Warming filter (81)
        [0x00, 0x6D, 0xFF], // Cooling filter (80)
        [0x00, 0x5D, 0xFF], // Cooling filter (LBB)
        [0x00, 0xB5, 0xFF], // Cooling filter (82)
        [0xEA, 0x1B, 0x00], // Red
        [0xF0, 0x7B, 0x00], // Orange
        [0xF0, 0xE1, 0x00], // Yellow
        [0x19, 0xF0, 0x00], // Green
        [0x00, 0xE5, 0xF0], // Cyan
        [0x00, 0x22, 0xF0], // Blue
        [0x97, 0x00, 0xF0], // Violet
        [0xFF, 0x00, 0xE1], // Magenta
        [0xAC, 0x7A, 0x33], // Sepia
        [0xFF, 0x00, 0x00], // Deep red
        [0x00, 0x00, 0xFF], // Deep blue
        [0x00, 0xFF, 0x00], // Deep emerald
        [0xFF, 0xD5, 0x00], // Deep yellow
        [0x00, 0xC1, 0xB1], // Underwater
    ];

    /// The scene-linear filter colour, the density, the luminosity flag and the
    /// mix (docs/impl/effect-registry.md §2.4). The sRGB decode happens here,
    /// once, so neither kernel carries a transfer function.
    #[must_use]
    pub fn packed(self) -> cpu::PhotoFilterParams {
        let rgb = if self.filter == Self::CUSTOM {
            [self.colour[0], self.colour[1], self.colour[2]]
        } else {
            let b = Self::FILTERS[(self.filter as usize).min(Self::FILTERS.len() - 1)];
            [
                crate::pixels::srgb_decode(b[0]),
                crate::pixels::srgb_decode(b[1]),
                crate::pixels::srgb_decode(b[2]),
            ]
        };
        cpu::PhotoFilterParams {
            filter: rgb,
            density: (self.density / 100.0).clamp(0.0, 1.0),
            // A float rather than a bool so the kernel multiplies instead of
            // branching.
            preserve: f32::from(u8::from(self.preserve_luminosity)),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Photo filter's behaviour.
pub struct PhotoFilterDef;

impl EffectDef for PhotoFilterDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PhotoFilter as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        cpu::photo_filter(rgba, &PhotoFilter::read(p).packed());
    }
}
