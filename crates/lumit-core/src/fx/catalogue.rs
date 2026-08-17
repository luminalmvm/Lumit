//! Registration: the list of effects this build has (docs/impl/
//! effect-registry.md §2.6).
//!
//! **In plain terms.** This is the whole of "adding an effect to Lumit": one
//! line, naming the thing that carries the effect's behaviour. The order here is
//! the order the Add-effect menu, the command palette and the preset browser
//! show (K-137), so it is deliberately a written list rather than something
//! assembled at start-up in whatever order the linker happened to choose.
//!
//! Effects that are not known when Lumit is compiled — OFX plugins (docs/12),
//! and in time the user's own — arrive through the same [`EffectDef`](crate::fx::
//! EffectDef) trait object at run time. That is the seam this arrangement exists
//! for; nothing here is a closed set any more.
//!
//! The migration to this list is in progress (docs/impl/effect-registry.md §6):
//! effects that have not moved yet are still declared in `builtins.rs` and
//! resolved through `resolved.rs`. `BUILTINS` remains the union of both while
//! that lasts, and a test holds the generated declaration and the hand-written
//! one to be identical for every effect that has moved.

use super::effects::{
    accumulation_mb::AccumulationMbDef, block_glitch::BlockGlitchDef, blur::BlurDef,
    chromatic_aberration::ChromaticAberrationDef, colour_balance::ColourBalanceDef,
    contrast::ContrastDef, datamosh::DatamoshDef, directional_blur::DirectionalBlurDef,
    dof::DofDef, echo::EchoDef, exposure::ExposureDef, flash::FlashDef, gamma::GammaDef,
    glow::GlowDef, hue_shift::HueShiftDef, invert::InvertDef, light_wrap::LightWrapDef,
    lut::LutDef, matte_key::MatteKeyDef, motion_blur::MotionBlurDef,
    posterize_time::PosterizeTimeDef, radial_blur::RadialBlurDef, rgb_split::RgbSplitDef,
    saturation::SaturationDef, scanlines::ScanlinesDef, sharpen::SharpenDef,
    sharpen_simple::SharpenSimpleDef, sprite_flare::SpriteFlareDef, temperature::TemperatureDef,
    tint::TintDef, transform::TransformDef, vibrancy::VibrancyDef, vignette::VignetteDef,
};

crate::catalogue![
    BlurDef,
    DirectionalBlurDef,
    RadialBlurDef,
    SharpenDef,
    SharpenSimpleDef,
    SpriteFlareDef,
    LightWrapDef,
    RgbSplitDef,
    ChromaticAberrationDef,
    FlashDef,
    ColourBalanceDef,
    SaturationDef,
    VibrancyDef,
    VignetteDef,
    ExposureDef,
    HueShiftDef,
    ContrastDef,
    GammaDef,
    TemperatureDef,
    LutDef,
    DofDef,
    TransformDef,
    GlowDef,
    BlockGlitchDef,
    ScanlinesDef,
    DatamoshDef,
    EchoDef,
    PosterizeTimeDef,
    AccumulationMbDef,
    MotionBlurDef,
    MatteKeyDef,
    InvertDef,
    TintDef,
];
