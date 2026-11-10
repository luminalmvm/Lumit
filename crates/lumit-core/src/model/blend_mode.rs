//! The blend modes' menu order and display names.

use super::BlendMode;

impl BlendMode {
    /// Every blend mode in After Effects' menu order, grouped
    /// darken → lighten → contrast → comparative → component. The
    /// single source of truth for the layer dropdown and the effect Mode
    /// param (T21), so the two never drift.
    pub const ALL: &'static [BlendMode] = &[
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColourBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColour,
        BlendMode::Add,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColourDodge,
        BlendMode::LighterColour,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::LinearLight,
        BlendMode::VividLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Colour,
        BlendMode::Luminosity,
    ];

    /// Every mode's display name, in [`Self::ALL`]'s order — the same words
    /// [`Self::name`] returns, as one `'static` list so a schema's Choice
    /// parameter can offer the layer modes verbatim (the injected effect
    /// Blend row). `blend_mode_all_is_complete_and_named` holds the two
    /// in step.
    pub const NAMES: &'static [&'static str] = &[
        "Normal",
        "Darken",
        "Multiply",
        "Colour burn",
        "Linear burn",
        "Darker colour",
        "Add",
        "Lighten",
        "Screen",
        "Colour dodge",
        "Lighter colour",
        "Overlay",
        "Soft light",
        "Hard light",
        "Linear light",
        "Vivid light",
        "Pin light",
        "Hard mix",
        "Difference",
        "Exclusion",
        "Subtract",
        "Divide",
        "Hue",
        "Saturation",
        "Colour",
        "Luminosity",
    ];

    /// The mode's display name (British English, sentence case — docs/15).
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Darken => "Darken",
            BlendMode::Multiply => "Multiply",
            BlendMode::ColourBurn => "Colour burn",
            BlendMode::LinearBurn => "Linear burn",
            BlendMode::DarkerColour => "Darker colour",
            BlendMode::Add => "Add",
            BlendMode::Lighten => "Lighten",
            BlendMode::Screen => "Screen",
            BlendMode::ColourDodge => "Colour dodge",
            BlendMode::LighterColour => "Lighter colour",
            BlendMode::Overlay => "Overlay",
            BlendMode::SoftLight => "Soft light",
            BlendMode::HardLight => "Hard light",
            BlendMode::LinearLight => "Linear light",
            BlendMode::VividLight => "Vivid light",
            BlendMode::PinLight => "Pin light",
            BlendMode::HardMix => "Hard mix",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Colour => "Colour",
            BlendMode::Luminosity => "Luminosity",
        }
    }
}
