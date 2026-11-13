//! Luma fade: the picture goes away in order of brightness, darkest first.
//!
//! The brightness is read from the Matte row when one is set, and from the
//! layer's own picture as it reaches the effect when it isn't, so a fresh
//! Luma fade fades the layer by its own luminance. Invert, beside the Matte,
//! makes the brightest parts go first.
//!
//! With no Matte, Pattern can stand in for the picture: noise, cells,
//! diamonds, stripes or the distance from the centre, square at any frame
//! shape. Picture, the default, is the fade as it always was.

use crate::fx::{
    builtins::MAP_PLACEMENT_OPTIONS, cpu::LUMA, noise::hash01, noise::value3, params::Params,
    presets::EffectPreset, presets::PresetValue, registry::EffectDef, registry::EffectMetadata,
    schema::EffectSchema, schema::EnabledCond, schema::EnabledWhen, schema::MATTE_INVERT_ID,
};
use lumit_fx_macros::Effect;

/// Map placement means nothing until a Matte layer is picked, and Scale
/// nothing while the brightness comes from the picture.
pub const LUMA_FADE_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "matte_placement",
        on: "matte",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "pattern_scale",
        on: "pattern",
        cond: EnabledCond::ChoiceIsNot(0),
    },
];

/// Luma fade's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "luma_fade",
    label = "Luma fade",
    version = 1,
    category = Transition,
    cost = Trivial,
    roi = Exact,
    // The picture is scaled by how much of each pixel is kept, all four
    // channels, as the other wipes do.
    premultiplied = true,
    matte = (
        "matte",
        "is where the brightness is read from: the darkest parts of the matte \
         go first, and with no matte set the layer fades by its own brightness",
    ),
    // The kernel reads the matte's brightness itself, so it can read the
    // layer's own picture the same way.
    matte_channel = false,
    enabled_when = LUMA_FADE_ENABLED_WHEN,
)]
pub struct LumaFade {
    /// How far the fade has got, per cent. 50 by default so the effect shows
    /// something when it's added, as Linear wipe's does. 0 keeps everything.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub completion: f32,

    /// How wide the edge between gone and kept is, per cent of the brightness
    /// range. 0 is a hard edge, which shimmers on footage, so a fresh one is a
    /// little soft.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 10.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub softness: f32,

    /// Where the brightness comes from when no Matte is set. Picture is the
    /// layer's own; the rest are drawn over the frame. A project saved before
    /// this row existed reads as Picture.
    #[choice(
        options = ["Picture", "Noise", "Cells", "Diamonds", "Stripes", "Centre"],
        default = 0
    )]
    pub pattern: u32,

    /// How many of the pattern's cells fit across the picture's shorter side.
    #[counter(
        label = "Scale",
        min = 1,
        max = 64,
        default = 8,
        hard_min = 1,
        hard_max = 64,
        unit = Raw
    )]
    pub pattern_scale: i32,

    /// Where the Matte layer's picture sits on this one.
    #[choice(label = "Map placement", options = *MAP_PLACEMENT_OPTIONS, default = 0)]
    pub matte_placement: u32,

    /// The host-uniform Mix every effect ends with, per cent.
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

/// One resolved Luma fade, reduced to what both paths read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LumaFadeParams {
    /// Completion ÷ 100.
    pub completion: f32,
    /// Softness as a share of the 0..1 brightness range, floored above zero.
    pub band: f32,
    /// The brightest parts go first instead of the darkest.
    pub invert: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// The Pattern row: 0 reads the picture.
    pub pattern: u32,
    /// Pattern cells a raster pixel, so the cells are square.
    pub step: f32,
    /// The picture's centre, raster pixels.
    pub centre: [f32; 2],
    /// One over the distance from the centre to a corner, raster pixels.
    pub reach: f32,
}

impl LumaFade {
    /// The numbers both kernels read on a `w` by `h` raster. `invert` is the
    /// Matte row's own switch, read out of the bag by the caller, as
    /// Displacement map's is.
    #[must_use]
    pub fn packed(self, invert: bool, w: u32, h: u32) -> LumaFadeParams {
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        LumaFadeParams {
            completion: (self.completion / 100.0).clamp(0.0, 1.0),
            // Floored so a hard edge is a steep ramp rather than a divide by
            // zero.
            band: (self.softness / 100.0).clamp(0.0, 1.0).max(1e-3),
            invert,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            pattern: self.pattern.min(5),
            step: self.pattern_scale.clamp(1, 64) as f32 / w.min(h),
            centre: [w * 0.5, h * 0.5],
            reach: 2.0 / w.hypot(h),
        }
    }
}

/// How bright a premultiplied pixel reads to the fade, 0..1: its Rec. 709
/// luma with the alpha taken back out, on the sRGB curve, so a ramp made by
/// eye fades evenly and a map from After Effects fades where it did there. A
/// see-through pixel reads as black. The WGSL twin is the same expression.
#[must_use]
pub fn luma_fade_brightness(px: &[f32]) -> f32 {
    let y = px[0] * LUMA[0] + px[1] * LUMA[1] + px[2] * LUMA[2];
    let y = if px[3] > 0.0 { y / px[3] } else { 0.0 };
    let y = y.clamp(0.0, 1.0);
    if y <= 0.003_130_8 {
        12.92 * y
    } else {
        1.055 * y.powf(1.0 / 2.4) - 0.055
    }
}

/// The brightness the pattern gives the pixel at `x`, `y`, 0..1, or `None`
/// when it is Picture. The WGSL twin is the same expression in the same order.
#[must_use]
pub fn luma_fade_pattern(x: u32, y: u32, p: &LumaFadeParams) -> Option<f32> {
    let dx = x as f32 + 0.5 - p.centre[0];
    let dy = y as f32 + 0.5 - p.centre[1];
    let (u, v) = (dx * p.step, dy * p.step);
    let l = match p.pattern {
        0 => return None,
        // Two octaves, pulled out a little so the fade does not sit still at
        // either end.
        1 => {
            let n =
                0.65 * value3(0, 0, u, v, 0.0, 0) + 0.35 * value3(0, 1, u * 2.0, v * 2.0, 0.0, 0);
            0.5 + 0.9 * n
        }
        2 => hash01(0, 2, u.floor() as i32, v.floor() as i32, 0),
        3 => (u - u.floor() - 0.5).abs() + (v - v.floor() - 0.5).abs(),
        4 => u - u.floor(),
        _ => (dx * dx + dy * dy).sqrt() * p.reach,
    };
    Some(l.clamp(0.0, 1.0))
}

/// How much of a pixel of brightness `l` the fade keeps, 0 to 1. The edge
/// travels half a band past each end of 0..1, so Completion 0 keeps
/// everything exactly and 100 removes everything exactly.
#[must_use]
pub fn luma_fade_keep(l: f32, p: &LumaFadeParams) -> f32 {
    let l = if p.invert { 1.0 - l } else { l };
    let edge = -0.5 * p.band + p.completion * (1.0 + p.band);
    ((l - edge) / p.band + 0.5).clamp(0.0, 1.0)
}

/// The CPU oracle on a raster `w` wide. The brightness is `matte`'s where one
/// is given, and the pattern's or the pixel's own where the matte is empty or
/// too short.
pub fn luma_fade(rgba: &mut [f32], w: u32, p: &LumaFadeParams, matte: &[f32]) {
    let w = w.max(1) as usize;
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i % w) as u32, (i / w) as u32);
        let l = match matte.get(i * 4..i * 4 + 4) {
            Some(m) => luma_fade_brightness(m),
            None => luma_fade_pattern(x, y, p).unwrap_or_else(|| luma_fade_brightness(px)),
        };
        // The same form as Linear wipe's, so a kept pixel scales by exactly 1.
        let f = 1.0 - p.mix * (1.0 - luma_fade_keep(l, p));
        for c in px {
            *c *= f;
        }
    }
}

/// Luma fade's behaviour.
pub struct LumaFadeDef;

impl EffectDef for LumaFadeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LumaFade as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [EffectPreset] {
        &[
            EffectPreset {
                name: "Luma reveal",
                values: &[
                    (
                        "completion",
                        PresetValue::KeysOut(&[(0.0, 100.0), (1.0, 0.0)]),
                    ),
                    ("softness", PresetValue::Float(25.0)),
                ],
            },
            EffectPreset {
                name: "Luma hide",
                values: &[
                    (
                        "completion",
                        PresetValue::KeysOut(&[(0.0, 0.0), (1.0, 100.0)]),
                    ),
                    ("softness", PresetValue::Float(25.0)),
                ],
            },
            EffectPreset {
                name: "Hard hide",
                values: &[
                    (
                        "completion",
                        PresetValue::KeysOut(&[(0.0, 0.0), (1.0, 100.0)]),
                    ),
                    ("softness", PresetValue::Float(0.0)),
                ],
            },
        ]
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let invert = p.bool(MATTE_INVERT_ID, false);
        luma_fade(rgba, w, &LumaFade::read(p).packed(invert, w, h), &[]);
    }
}
