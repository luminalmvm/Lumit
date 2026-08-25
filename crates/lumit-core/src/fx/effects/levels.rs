//! Levels (docs/08 §3.31): input black/white, gamma and output black/white,
//! per channel — the five numbers an eye can aim, where Curves is a shape.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamGroup, Params};
use lumit_fx_macros::Effect;

/// Levels' four channel groups (docs/08 §3.31), the same shape Curves uses:
/// Master open, the three colour channels closed, each a contiguous run of the
/// schema's `params`.
pub const LEVELS_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Master",
        params: &[
            "master_in_black",
            "master_in_white",
            "master_gamma",
            "master_out_black",
            "master_out_white",
        ],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Red",
        params: &[
            "red_in_black",
            "red_in_white",
            "red_gamma",
            "red_out_black",
            "red_out_white",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Green",
        params: &[
            "green_in_black",
            "green_in_white",
            "green_gamma",
            "green_out_black",
            "green_out_white",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Blue",
        params: &[
            "blue_in_black",
            "blue_in_white",
            "blue_gamma",
            "blue_out_black",
            "blue_out_white",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Levels' controls: input black and white, gamma, and output black and white
/// on each of Master, Red, Green and Blue.
///
/// Defaults are neutral, the grade family's sanctioned exception to the
/// "no no-op default" rule (docs/08 §3.10). Gamma floors at 0.01 exactly as
/// the one-knob Gamma effect does, so its reciprocal stays finite; the four
/// level values are unbounded either way by typing, because a black point
/// below zero and a white point above one are both real moves in scene-linear
/// light.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "levels",
    label = "Levels",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: the gamma power is non-linear, so it does not commute with
    // premultiplied alpha.
    premultiplied = false,
    groups = LEVELS_GROUPS,
)]
pub struct Levels {
    /// The Master input value that maps to Output black.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Input black", unit = Raw)]
    pub master_in_black: f32,
    /// The Master input value that maps to Output white.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Input white", unit = Raw)]
    pub master_in_white: f32,
    /// Master's mid-tone bend; above 1 lifts, below 1 lowers.
    #[slider(min = 0.1, max = 4.0, default = 1.0, hard_min = 0.01, label = "Gamma", unit = Raw)]
    pub master_gamma: f32,
    /// What Master's input black comes out as.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Output black", unit = Raw)]
    pub master_out_black: f32,
    /// What Master's input white comes out as.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Output white", unit = Raw)]
    pub master_out_white: f32,

    /// The Red input value that maps to its Output black.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Input black", unit = Raw)]
    pub red_in_black: f32,
    /// The Red input value that maps to its Output white.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Input white", unit = Raw)]
    pub red_in_white: f32,
    /// Red's mid-tone bend.
    #[slider(min = 0.1, max = 4.0, default = 1.0, hard_min = 0.01, label = "Gamma", unit = Raw)]
    pub red_gamma: f32,
    /// What Red's input black comes out as.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Output black", unit = Raw)]
    pub red_out_black: f32,
    /// What Red's input white comes out as.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Output white", unit = Raw)]
    pub red_out_white: f32,

    /// The Green input value that maps to its Output black.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Input black", unit = Raw)]
    pub green_in_black: f32,
    /// The Green input value that maps to its Output white.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Input white", unit = Raw)]
    pub green_in_white: f32,
    /// Green's mid-tone bend.
    #[slider(min = 0.1, max = 4.0, default = 1.0, hard_min = 0.01, label = "Gamma", unit = Raw)]
    pub green_gamma: f32,
    /// What Green's input black comes out as.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Output black", unit = Raw)]
    pub green_out_black: f32,
    /// What Green's input white comes out as.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Output white", unit = Raw)]
    pub green_out_white: f32,

    /// The Blue input value that maps to its Output black.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Input black", unit = Raw)]
    pub blue_in_black: f32,
    /// The Blue input value that maps to its Output white.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Input white", unit = Raw)]
    pub blue_in_white: f32,
    /// Blue's mid-tone bend.
    #[slider(min = 0.1, max = 4.0, default = 1.0, hard_min = 0.01, label = "Gamma", unit = Raw)]
    pub blue_gamma: f32,
    /// What Blue's input black comes out as.
    #[slider(min = 0.0, max = 1.0, default = 0.0, label = "Output black", unit = Raw)]
    pub blue_out_black: f32,
    /// What Blue's input white comes out as.
    #[slider(min = 0.0, max = 1.0, default = 1.0, label = "Output white", unit = Raw)]
    pub blue_out_white: f32,

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

impl Levels {
    /// The five rows the kernel maps with, indexed `[row][channel]` with
    /// channel 0 Master and 1..3 R/G/B — input black, the **reciprocal** input
    /// span, the **reciprocal** gamma, output black, the output span — plus
    /// the mix (docs/impl/effect-registry.md §2.4).
    ///
    /// Both reciprocals are taken here, once, so neither render path divides
    /// per pixel and the two cannot disagree in the last bit. The input span
    /// floors at 1e-4, so a white point dragged below a black point saturates
    /// instead of dividing by zero — which is what the picture should do.
    #[must_use]
    pub fn packed(self) -> ([[f32; 4]; 5], f32) {
        let channels = [
            [
                self.master_in_black,
                self.master_in_white,
                self.master_gamma,
                self.master_out_black,
                self.master_out_white,
            ],
            [
                self.red_in_black,
                self.red_in_white,
                self.red_gamma,
                self.red_out_black,
                self.red_out_white,
            ],
            [
                self.green_in_black,
                self.green_in_white,
                self.green_gamma,
                self.green_out_black,
                self.green_out_white,
            ],
            [
                self.blue_in_black,
                self.blue_in_white,
                self.blue_gamma,
                self.blue_out_black,
                self.blue_out_white,
            ],
        ];
        let mut r = [[0.0f32; 4]; 5];
        for (c, v) in channels.iter().enumerate() {
            r[0][c] = v[0];
            r[1][c] = 1.0 / (v[1] - v[0]).max(1e-4);
            r[2][c] = 1.0 / v[2].max(0.01);
            r[3][c] = v[3];
            r[4][c] = v[4] - v[3];
        }
        (r, (self.mix / 100.0).clamp(0.0, 1.0))
    }
}

/// Levels' behaviour.
pub struct LevelsDef;

impl EffectDef for LevelsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Levels as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (r, mix) = Levels::read(p).packed();
        cpu::levels(rgba, r, mix);
    }
}
