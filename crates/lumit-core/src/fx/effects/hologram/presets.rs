//! Hologram's built-in looks: films, since the subject is the layer.

use crate::fx::presets::{EffectPreset, PresetValue};

/// A reflection hologram in one green over a grid; an embossed rainbow
/// hologram; a stereogram of stepped views; a security seal with line
/// patterns; holospot's sticker film, a small dark film with the seal
/// filling it and the subject a quarter of its width deep; and a subject
/// near the film over a Backdrop layer far behind it.
pub const HOLOGRAM_LOOKS: &[EffectPreset] = &[
    EffectPreset {
        name: "Reflection hologram",
        values: &[],
    },
    EffectPreset {
        name: "Rainbow hologram",
        values: &[
            ("hologram_type", PresetValue::Choice(super::RAINBOW)),
            ("glare", PresetValue::Float(40.0)),
        ],
    },
    EffectPreset {
        name: "Stereogram",
        values: &[
            ("recording", PresetValue::Choice(super::STEREOGRAM)),
            ("views", PresetValue::Float(20.0)),
            ("view_blend", PresetValue::Float(25.0)),
        ],
    },
    EffectPreset {
        name: "Security seal",
        values: &[
            ("hologram_type", PresetValue::Choice(super::RAINBOW)),
            ("backdrop", PresetValue::Choice(3)),
            ("pattern_size", PresetValue::Float(60.0)),
            ("pattern_strength", PresetValue::Float(1.5)),
            ("glare", PresetValue::Float(40.0)),
        ],
    },
    EffectPreset {
        name: "Sticker",
        values: &[
            ("hologram_type", PresetValue::Choice(super::RAINBOW)),
            ("backdrop", PresetValue::Choice(3)),
            ("recording", PresetValue::Choice(super::STEREOGRAM)),
            ("views", PresetValue::Float(16.0)),
            ("view_blend", PresetValue::Float(50.0)),
            ("slit", PresetValue::Float(4.0)),
            ("glare", PresetValue::Float(15.0)),
            ("speckle", PresetValue::Float(30.0)),
            ("sheet_width", PresetValue::Float(30.0)),
            ("subject_depth", PresetValue::Float(-6.0)),
            ("backdrop_depth", PresetValue::Float(-24.0)),
        ],
    },
    EffectPreset {
        name: "Deep backdrop",
        values: &[
            ("backdrop", PresetValue::Choice(0)),
            ("subject_depth", PresetValue::Float(-3.0)),
            ("backdrop_depth", PresetValue::Float(-45.0)),
            ("subject_colour", PresetValue::Choice(1)),
        ],
    },
];
