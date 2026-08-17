//! Registration: the list of effects this build has (docs/impl/
//! effect-registry.md §2.6).
//!
//! **In plain terms.** This is the whole of "adding an effect to Lumit": one
//! line, naming the thing that carries the effect's behaviour and the block that
//! declares its controls. The order here is the order the Add-effect menu, the
//! command palette and the preset browser show (K-137), so it is deliberately a
//! written list rather than something assembled at start-up in whatever order
//! the linker happened to choose.
//!
//! Effects that are not known when Lumit is compiled — OFX plugins (docs/12),
//! and in time the user's own — arrive through the same [`EffectDef`](crate::fx::
//! EffectDef) trait object at run time. That is the seam this arrangement exists
//! for; nothing here is a closed set any more.
//!
//! Both halves of the catalogue come from this one list: `BUILTIN_DEFS`, the
//! behaviours the frame walk dispatches through, and `BUILTINS`, the
//! declarations the menu and the bridge read. Each line's left side is the
//! effect's behaviour, its right side the parameter block `#[derive(Effect)]`
//! generated the declaration from.

use super::effects::{
    accumulation_mb::{AccumulationMb, AccumulationMbDef},
    block_glitch::{BlockGlitch, BlockGlitchDef},
    blur::{Blur, BlurDef},
    chromatic_aberration::{ChromaticAberration, ChromaticAberrationDef},
    colour_balance::{ColourBalance, ColourBalanceDef},
    contrast::{Contrast, ContrastDef},
    datamosh::{Datamosh, DatamoshDef},
    directional_blur::{DirectionalBlur, DirectionalBlurDef},
    dof::{Dof, DofDef},
    echo::{Echo, EchoDef},
    exposure::{Exposure, ExposureDef},
    flash::{Flash, FlashDef},
    gamma::{Gamma, GammaDef},
    glow::{Glow, GlowDef},
    hue_shift::{HueShift, HueShiftDef},
    invert::{Invert, InvertDef},
    lens_flare::{LensFlare, LensFlareDef},
    light_wrap::{LightWrap, LightWrapDef},
    lut::{Lut, LutDef},
    matte_key::{MatteKey, MatteKeyDef},
    motion_blur::{MotionBlur, MotionBlurDef},
    posterize_time::{PosterizeTime, PosterizeTimeDef},
    radial_blur::{RadialBlur, RadialBlurDef},
    rgb_split::{RgbSplit, RgbSplitDef},
    saturation::{Saturation, SaturationDef},
    scanlines::{Scanlines, ScanlinesDef},
    shake::{Shake, ShakeDef},
    sharpen::{Sharpen, SharpenDef},
    sharpen_simple::{SharpenSimple, SharpenSimpleDef},
    sprite_flare::{SpriteFlare, SpriteFlareDef},
    temperature::{Temperature, TemperatureDef},
    tint::{Tint, TintDef},
    transform::{Transform, TransformDef},
    vibrancy::{Vibrancy, VibrancyDef},
    vignette::{Vignette, VignetteDef},
};

crate::catalogue![
    BlurDef => Blur,
    DirectionalBlurDef => DirectionalBlur,
    RadialBlurDef => RadialBlur,
    SharpenDef => Sharpen,
    SharpenSimpleDef => SharpenSimple,
    SpriteFlareDef => SpriteFlare,
    LightWrapDef => LightWrap,
    RgbSplitDef => RgbSplit,
    ChromaticAberrationDef => ChromaticAberration,
    FlashDef => Flash,
    ColourBalanceDef => ColourBalance,
    SaturationDef => Saturation,
    VibrancyDef => Vibrancy,
    VignetteDef => Vignette,
    ExposureDef => Exposure,
    HueShiftDef => HueShift,
    ContrastDef => Contrast,
    GammaDef => Gamma,
    TemperatureDef => Temperature,
    LutDef => Lut,
    DofDef => Dof,
    TransformDef => Transform,
    GlowDef => Glow,
    ShakeDef => Shake,
    BlockGlitchDef => BlockGlitch,
    ScanlinesDef => Scanlines,
    DatamoshDef => Datamosh,
    EchoDef => Echo,
    PosterizeTimeDef => PosterizeTime,
    AccumulationMbDef => AccumulationMb,
    MotionBlurDef => MotionBlur,
    MatteKeyDef => MatteKey,
    InvertDef => Invert,
    TintDef => Tint,
    LensFlareDef => LensFlare,
];
