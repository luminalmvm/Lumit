//! Holographic foil's built-in looks. Each starts from a fresh effect, so
//! Tint and Lighting stay Auto unless a look names them.

use crate::fx::presets::{EffectPreset, PresetValue};

/// The default card; a compact disc; a sunburst; rainbow foil under gold;
/// cracked ice under gold, clear or blue lacquer; soft satin foil; flakes
/// under a blue or clear binder; sparkle dots under sky-blue lacquer;
/// holospot's gilded vortex; and a logo pressed into rainbow foil.
pub const HOLOGRAPHIC_FOIL_LOOKS: &[EffectPreset] = &[
    EffectPreset {
        name: "Card rainbow",
        values: &[],
    },
    EffectPreset {
        name: "Compact disc",
        values: COMPACT_DISC,
    },
    EffectPreset {
        name: "Sunburst",
        values: SUNBURST,
    },
    EffectPreset {
        name: "Gold rainbow foil",
        values: GOLD_RAINBOW,
    },
    EffectPreset {
        name: "Gold shatter",
        values: SHATTER_GOLD,
    },
    EffectPreset {
        name: "Silver shatter",
        values: SHATTER_SILVER,
    },
    EffectPreset {
        name: "Ice blue",
        values: ICE_BLUE,
    },
    EffectPreset {
        name: "Soft pastel satin",
        values: SATIN,
    },
    EffectPreset {
        name: "Nebula glitter",
        values: NEBULA,
    },
    EffectPreset {
        name: "Silver glitter",
        values: &[
            ("material", PresetValue::Choice(super::GLITTER)),
            ("flake_size", PresetValue::Float(320.0)),
            ("depth", PresetValue::Float(100.0)),
            ("coverage", PresetValue::Float(65.0)),
            ("lacquer", PresetValue::Float(1.5)),
            ("tint", PresetValue::Choice(0)),
            ("coat_roughness", PresetValue::Float(0.04)),
            ("lighting", PresetValue::Choice(0)),
        ],
    },
    EffectPreset {
        name: "Sky sparkle",
        values: SKY,
    },
    EffectPreset {
        name: "Gilded vortex",
        values: &[
            ("material", PresetValue::Choice(super::VORTEX)),
            ("pitch", PresetValue::Float(1250.0)),
            ("depth", PresetValue::Float(45.0)),
            ("fan", PresetValue::Float(1.5)),
            ("softness", PresetValue::Float(0.25)),
            ("lacquer", PresetValue::Float(1.5)),
            ("coat_roughness", PresetValue::Float(0.045)),
        ],
    },
    EffectPreset {
        name: "Pressed logo",
        values: &[
            ("print", PresetValue::Choice(1)),
            (
                "relief_source",
                PresetValue::Choice(super::surface::BEVEL_SOURCE),
            ),
            ("bevel_width", PresetValue::Float(1.5)),
            ("bevel_height", PresetValue::Float(180.0)),
        ],
    },
];

/// Fine rings about the middle with almost no fan, on smooth flat metal
/// under one spot: a thin rainbow streak through the middle of a mirror.
const COMPACT_DISC: &[(&str, PresetValue)] = &[
    ("pitch", PresetValue::Float(1600.0)),
    ("bow", PresetValue::Float(0.0)),
    ("depth", PresetValue::Float(90.0)),
    ("fan", PresetValue::Float(1.0)),
    ("orientation_jitter", PresetValue::Float(0.1)),
    ("pitch_jitter", PresetValue::Float(0.2)),
    ("relief_source", PresetValue::Choice(7)),
    ("lighting", PresetValue::Choice(1)),
    ("exposure", PresetValue::Float(0.7)),
];

/// Grooves running out from the middle, so the colour lies in rings round
/// it.
const SUNBURST: &[(&str, PresetValue)] = &[
    ("pattern", PresetValue::Choice(2)),
    ("pitch", PresetValue::Float(1200.0)),
    ("fan", PresetValue::Float(4.0)),
];

/// The card's rings, deeper, under a gold dye: warm rainbow arms over gold
/// metal.
const GOLD_RAINBOW: &[(&str, PresetValue)] = &[
    ("depth", PresetValue::Float(70.0)),
    ("tint", PresetValue::Choice(1)),
];

/// Holospot's gold shatter: shards with deeper grooves on a part-plain foil,
/// a rougher metal and a gold dye.
const SHATTER_GOLD: &[(&str, PresetValue)] = &[
    ("material", PresetValue::Choice(super::SHATTER)),
    ("depth", PresetValue::Float(95.0)),
    ("coverage", PresetValue::Float(70.0)),
    ("fan", PresetValue::Float(5.0)),
    ("softness", PresetValue::Float(0.45)),
    ("lacquer", PresetValue::Float(1.5)),
    ("tint", PresetValue::Choice(1)),
    ("coat_roughness", PresetValue::Float(0.05)),
];

/// The same shards under a clear lacquer.
const SHATTER_SILVER: &[(&str, PresetValue)] = &[
    ("material", PresetValue::Choice(super::SHATTER)),
    ("depth", PresetValue::Float(95.0)),
    ("coverage", PresetValue::Float(70.0)),
    ("fan", PresetValue::Float(5.0)),
    ("softness", PresetValue::Float(0.45)),
    ("lacquer", PresetValue::Float(1.5)),
    ("tint", PresetValue::Choice(0)),
    ("coat_roughness", PresetValue::Float(0.05)),
];

/// Cracked ice under a blue dye, with larger shards.
const ICE_BLUE: &[(&str, PresetValue)] = &[
    ("material", PresetValue::Choice(super::SHATTER)),
    ("shard_size", PresetValue::Float(3.5)),
    ("depth", PresetValue::Float(80.0)),
    ("coverage", PresetValue::Float(70.0)),
    ("softness", PresetValue::Float(0.3)),
    ("tint", PresetValue::Choice(2)),
    ("coat_roughness", PresetValue::Float(0.03)),
];

/// Shallow grooves on brushed, softened metal: pale, wide bands with a satin
/// sheen.
const SATIN: &[(&str, PresetValue)] = &[
    ("depth", PresetValue::Float(35.0)),
    ("fan", PresetValue::Float(15.0)),
    ("softness", PresetValue::Float(1.2)),
    ("pitch_jitter", PresetValue::Float(2.0)),
    ("relief_source", PresetValue::Choice(1)),
    ("relief", PresetValue::Float(0.5)),
    ("coat_roughness", PresetValue::Float(0.15)),
];

/// Holospot's nebula: fine round flakes in two layers under a blue binder
/// over indigo ink, with many stars, seen close under the studio so the
/// flakes sparkle one by one.
const NEBULA: &[(&str, PresetValue)] = &[
    ("material", PresetValue::Choice(super::GLITTER)),
    ("flake_size", PresetValue::Float(140.0)),
    ("flake_shape", PresetValue::Choice(2)),
    ("flake_tilt", PresetValue::Float(2.7)),
    ("flake_curve", PresetValue::Float(0.05)),
    ("pitch_min", PresetValue::Float(870.0)),
    ("pitch_max", PresetValue::Float(1180.0)),
    ("star_density", PresetValue::Float(30.0)),
    (
        "base_colour",
        PresetValue::Colour([0.016, 0.0075, 0.115, 1.0]),
    ),
    ("depth", PresetValue::Float(146.0)),
    ("coverage", PresetValue::Float(57.0)),
    ("lacquer", PresetValue::Float(1.65)),
    ("tint", PresetValue::Choice(3)),
    ("tint_pass", PresetValue::Choice(1)),
    ("tint_edge", PresetValue::Float(525.0)),
    ("tint_width", PresetValue::Float(150.0)),
    ("tint_floor", PresetValue::Float(7.0)),
    ("waviness", PresetValue::Float(0.57)),
    ("orange_peel", PresetValue::Float(1.2)),
    ("coat_roughness", PresetValue::Float(0.01)),
    ("sheet_width", PresetValue::Float(40.0)),
    ("lighting", PresetValue::Choice(0)),
];

/// Holospot's sky sparkle: dots sharing one grating over sand-matte metal,
/// under a sky-blue lacquer. Rings, so the band crosses the dots at any
/// turn of the sheet.
const SKY: &[(&str, PresetValue)] = &[
    ("material", PresetValue::Choice(super::SPARKLE)),
    ("depth", PresetValue::Float(55.0)),
    ("fan", PresetValue::Float(12.0)),
    ("softness", PresetValue::Float(0.15)),
    ("orientation_jitter", PresetValue::Float(2.0)),
    ("pitch_jitter", PresetValue::Float(1.5)),
    ("lacquer", PresetValue::Float(1.5)),
    ("coat_roughness", PresetValue::Float(0.05)),
];
