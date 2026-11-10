//! The byte a blend mode feeds into a frame key.

/// Stable one-byte tag per blend mode (never reuse a value — the key must
/// not change meaning across versions without an ALGO_VERSION bump).
pub(super) fn blend_tag(b: lumit_core::model::BlendMode) -> u8 {
    use lumit_core::model::BlendMode;
    match b {
        BlendMode::Normal => 0,
        BlendMode::Add => 1,
        BlendMode::Multiply => 2,
        BlendMode::Screen => 3,
        BlendMode::Overlay => 4,
        BlendMode::SoftLight => 5,
        BlendMode::HardLight => 6,
        BlendMode::Lighten => 7,
        BlendMode::Darken => 8,
        BlendMode::Subtract => 9,
        // The rest of the After Effects set (T24). Never reuse a tag —
        // the value is part of the frame-cache key.
        BlendMode::ColourBurn => 10,
        BlendMode::LinearBurn => 11,
        BlendMode::DarkerColour => 12,
        BlendMode::ColourDodge => 13,
        BlendMode::LighterColour => 14,
        BlendMode::LinearLight => 15,
        BlendMode::VividLight => 16,
        BlendMode::PinLight => 17,
        BlendMode::HardMix => 18,
        BlendMode::Difference => 19,
        BlendMode::Exclusion => 20,
        BlendMode::Divide => 21,
        BlendMode::Hue => 22,
        BlendMode::Saturation => 23,
        BlendMode::Colour => 24,
        BlendMode::Luminosity => 25,
    }
}
