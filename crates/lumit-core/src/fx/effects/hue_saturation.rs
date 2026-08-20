//! Hue and saturation (docs/08 §3.33): a master adjustment plus six colour
//! ranges, each hue, saturation and lightness — the AE Hue/Saturation
//! workhorse. The one-knob Hue shift (§3.17) stays: that is a
//! constant-luminance matrix rotation, and this is not.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamGroup, Params};
use lumit_fx_macros::Effect;

/// One band's twirl. Master opens; the six ranges start closed, because a
/// panel that opened all seven would be sixty rows tall.
const fn band(label: &'static str, params: &'static [&'static str], collapsed: bool) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// The seven bands, in the order the weights index them: master, then the six
/// ranges centred on red, yellow, green, cyan, blue and magenta.
pub const HUE_SATURATION_GROUPS: &[ParamGroup] = &[
    band(
        "Master",
        &["master_hue", "master_saturation", "master_lightness"],
        false,
    ),
    band(
        "Reds",
        &["reds_hue", "reds_saturation", "reds_lightness"],
        true,
    ),
    band(
        "Yellows",
        &["yellows_hue", "yellows_saturation", "yellows_lightness"],
        true,
    ),
    band(
        "Greens",
        &["greens_hue", "greens_saturation", "greens_lightness"],
        true,
    ),
    band(
        "Cyans",
        &["cyans_hue", "cyans_saturation", "cyans_lightness"],
        true,
    ),
    band(
        "Blues",
        &["blues_hue", "blues_saturation", "blues_lightness"],
        true,
    ),
    band(
        "Magentas",
        &["magentas_hue", "magentas_saturation", "magentas_lightness"],
        true,
    ),
];

/// Hue and saturation's controls: seven groups of three.
///
/// Master applies to every pixel. Each range applies by how close the pixel's
/// hue is to that range's centre — hat functions 120° wide centred every 60°,
/// so the six sum to exactly 1 for any hue and no boundary can be crossed —
/// and by how saturated the pixel already is, so a grey (whose hue reads 0°,
/// which is red) takes Master alone (docs/08 §3.33).
///
/// Neutral by default, the grade family's sanctioned exception to the
/// "no no-op default" rule (docs/08 §3.10). The rows repeat their labels
/// across the seven groups on purpose: the header says which colours, the row
/// says which adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "hue_saturation",
    label = "Hue and saturation",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: an HSV round trip is neither a scale nor linear, so it does not
    // commute with premultiplied alpha.
    premultiplied = false,
    groups = HUE_SATURATION_GROUPS,
)]
pub struct HueSaturation {
    /// Degrees round the colour wheel, applied to every pixel.
    #[dial(default = 0.0, step = 15.0, label = "Hue")]
    pub master_hue: f32,
    /// Per cent: 0 neutral, −100 grey, +100 doubled.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Saturation"
    )]
    pub master_saturation: f32,
    /// Per cent gain on the HSV value: −100 to black, +100 doubled.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Lightness"
    )]
    pub master_lightness: f32,

    /// Degrees, weighted toward hues near red.
    #[dial(default = 0.0, step = 15.0, label = "Hue")]
    pub reds_hue: f32,
    /// Per cent, weighted toward hues near red.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Saturation"
    )]
    pub reds_saturation: f32,
    /// Per cent, weighted toward hues near red.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Lightness"
    )]
    pub reds_lightness: f32,

    /// Degrees, weighted toward hues near yellow.
    #[dial(default = 0.0, step = 15.0, label = "Hue")]
    pub yellows_hue: f32,
    /// Per cent, weighted toward hues near yellow.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Saturation"
    )]
    pub yellows_saturation: f32,
    /// Per cent, weighted toward hues near yellow.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Lightness"
    )]
    pub yellows_lightness: f32,

    /// Degrees, weighted toward hues near green.
    #[dial(default = 0.0, step = 15.0, label = "Hue")]
    pub greens_hue: f32,
    /// Per cent, weighted toward hues near green.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Saturation"
    )]
    pub greens_saturation: f32,
    /// Per cent, weighted toward hues near green.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Lightness"
    )]
    pub greens_lightness: f32,

    /// Degrees, weighted toward hues near cyan.
    #[dial(default = 0.0, step = 15.0, label = "Hue")]
    pub cyans_hue: f32,
    /// Per cent, weighted toward hues near cyan.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Saturation"
    )]
    pub cyans_saturation: f32,
    /// Per cent, weighted toward hues near cyan.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Lightness"
    )]
    pub cyans_lightness: f32,

    /// Degrees, weighted toward hues near blue.
    #[dial(default = 0.0, step = 15.0, label = "Hue")]
    pub blues_hue: f32,
    /// Per cent, weighted toward hues near blue.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Saturation"
    )]
    pub blues_saturation: f32,
    /// Per cent, weighted toward hues near blue.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Lightness"
    )]
    pub blues_lightness: f32,

    /// Degrees, weighted toward hues near magenta.
    #[dial(default = 0.0, step = 15.0, label = "Hue")]
    pub magentas_hue: f32,
    /// Per cent, weighted toward hues near magenta.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Saturation"
    )]
    pub magentas_saturation: f32,
    /// Per cent, weighted toward hues near magenta.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        label = "Lightness"
    )]
    pub magentas_lightness: f32,

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

impl HueSaturation {
    /// The seven bands as `[hue, saturation %, lightness %, unused]`, master
    /// first, plus the mix (docs/impl/effect-registry.md §2.4). The fourth
    /// lane is padding: a band is a `vec4` in the uniform, which is what a
    /// uniform array of three floats would have become anyway.
    #[must_use]
    pub fn packed(self) -> ([[f32; 4]; 7], f32) {
        (
            [
                [
                    self.master_hue,
                    self.master_saturation,
                    self.master_lightness,
                    0.0,
                ],
                [
                    self.reds_hue,
                    self.reds_saturation,
                    self.reds_lightness,
                    0.0,
                ],
                [
                    self.yellows_hue,
                    self.yellows_saturation,
                    self.yellows_lightness,
                    0.0,
                ],
                [
                    self.greens_hue,
                    self.greens_saturation,
                    self.greens_lightness,
                    0.0,
                ],
                [
                    self.cyans_hue,
                    self.cyans_saturation,
                    self.cyans_lightness,
                    0.0,
                ],
                [
                    self.blues_hue,
                    self.blues_saturation,
                    self.blues_lightness,
                    0.0,
                ],
                [
                    self.magentas_hue,
                    self.magentas_saturation,
                    self.magentas_lightness,
                    0.0,
                ],
            ],
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Hue and saturation's behaviour.
pub struct HueSaturationDef;

impl EffectDef for HueSaturationDef {
    fn schema(&self) -> &'static EffectSchema {
        &<HueSaturation as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (bands, mix) = HueSaturation::read(p).packed();
        cpu::hue_saturation(rgba, bands, mix);
    }
}
