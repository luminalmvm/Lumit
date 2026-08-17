//! The GPU half of a migrated effect: name in, texture out
//! (docs/impl/effect-registry.md §2.5).
//!
//! **In plain terms.** An effect that has moved to the registry no longer has a
//! variant in the big enum, so `run_ops` cannot reach its kernel with a `match`
//! arm any more. This module is what it reaches instead: a small list of
//! wrappers, each naming one effect and knowing the single `FxEngine` call that
//! draws it. Looking one up by name gives something callable, so adding an
//! effect adds a line here rather than an arm there.
//!
//! **Why it lives in `lumit-render` and not beside the declaration.**
//! `lumit-gpu` only *dev*-depends on `lumit-core` (docs/05 crate table), so the
//! kernels cannot be named from the same file as the schema. `lumit-render`
//! depends on both, which makes it the only place the two halves can meet. The
//! join is by `match_name` string, and a typo there is a missing effect at run
//! time rather than a compile error — which is why
//! `every_migrated_effect_has_a_gpu_entry` is not optional.
//!
//! The wrappers are deliberately thin. None of them does arithmetic: each reads
//! the effect's own typed struct out of the resolved bag and asks it for the
//! numbers the kernel wants (`packed`), so the CPU reference and the WGSL kernel
//! multiply by values that came from one expression, not two.

use lumit_core::fx::effects;
use lumit_core::fx::{EffectMetadata, Params};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::fxops::LoadedLut;

type Tex = wgpu::Texture;

/// Which parallel input list an effect consumes a slot of
/// (docs/impl/effect-registry.md §2.5a, K-387).
///
/// **In plain terms.** A few effects need something the *render* prepared, not
/// something the user typed: a parsed `.cube`, another layer's picture, the
/// frames either side of this one, the motion field. Those arrive as lists
/// running alongside the stack, and which entry of a list belongs to which op is
/// settled by counting: the k-th LUT op takes the k-th cube. This says which
/// list an effect counts along, so `run_ops` can advance that counter and hand
/// the slot over without knowing anything else about the effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxKind {
    /// Nothing beside the op — every effect but a handful.
    None,
    /// The parallel LUT list (docs/08 §3.11): one parsed cube per `lut` op.
    Lut,
    /// The parallel layer-input list (docs/08 §3.22, docs/impl/layer-input.md):
    /// one rendered layer per consuming op, shared by Depth of field and Light
    /// wrap because `build.rs` enumerates them with one predicate.
    LayerInput,
    /// The Lens flare's pair (K-257, K-264): its Matte source and its custom
    /// prescription, counted together off one index.
    FlareInputs,
    /// The layer's decoded neighbour frames — the whole list, never per-op.
    Neighbours,
    /// The layer's decoded motion — the dense field and the neighbour frames
    /// beside it, both whole, never per-op.
    FlowField,
}

/// The borrowed slot itself, as [`crate::fxops::run_ops`] resolved it.
///
/// A missing input is `None` (or an empty list), which every effect that takes
/// one renders as a passthrough: an unset LUT, a dangling layer reference or a
/// dropped decode degrades the picture, it never faults
/// (14-ENGINEERING-RULES §4).
#[derive(Clone, Copy)]
pub enum AuxSlot<'a> {
    /// The effect declared [`AuxKind::None`].
    None,
    /// This op's parsed cube, or `None` when the file was unset, missing, 1D or
    /// unreadable.
    Lut(Option<&'a LoadedLut>),
    /// This op's layer input, already resolved against the picture the chain is
    /// carrying (so [`crate::fxops::LayerInput::ThisLayer`] is a real texture by
    /// the time it arrives here).
    LayerInput(Option<&'a Tex>),
    /// The flare's Matte source and its custom prescription (content hash and
    /// text), each absent on its own terms.
    FlareInputs {
        matte: Option<&'a Tex>,
        lens: Option<&'a (u64, String)>,
    },
    /// Every decoded neighbour frame, keyed by offset; empty unless the stack
    /// asked for a temporal window.
    Neighbours(&'a [(i32, Tex)]),
    /// The layer's decoded motion: the dense field at this raster if one was
    /// computed, and the neighbour frames it displaces. Both come off the one
    /// decode, and Datamosh reads both — the field to walk along, the −1 frame
    /// to drag — which is why they arrive together, as the flare's matte and its
    /// prescription do. Fast motion blur ignores the frames.
    FlowField {
        field: Option<&'a Tex>,
        neighbours: &'a [(i32, Tex)],
    },
}

impl<'a> AuxSlot<'a> {
    /// This op's cube. `None` for a missing slot *and* for a slot of the wrong
    /// kind — which cannot happen, since [`GpuEffect::aux`] is what chose the
    /// kind, and which is a passthrough rather than a panic if it ever does.
    pub fn lut(self) -> Option<&'a LoadedLut> {
        match self {
            AuxSlot::Lut(l) => l,
            _ => None,
        }
    }

    /// This op's layer input — the depth pass, the background plate — already
    /// resolved against the picture the chain is carrying. `None` for an unset,
    /// missing or cyclic reference: the labelled no-op.
    pub fn layer_input(self) -> Option<&'a Tex> {
        match self {
            AuxSlot::LayerInput(t) => t,
            _ => None,
        }
    }

    /// The decoded neighbour frames, empty when there are none — from either of
    /// the two kinds that carry them.
    pub fn neighbours(self) -> &'a [(i32, Tex)] {
        match self {
            AuxSlot::Neighbours(n) => n,
            AuxSlot::FlowField { neighbours, .. } => neighbours,
            _ => &[],
        }
    }

    /// The dense motion field, `None` when the decode computed none — a plain
    /// layer, or a dropped neighbour. The passthrough, never a fault.
    pub fn flow_field(self) -> Option<&'a Tex> {
        match self {
            AuxSlot::FlowField { field, .. } => field,
            _ => None,
        }
    }

    /// The Lens flare's Matte source and its custom prescription, each absent on
    /// its own terms: an unset or dangling matte detects no sources, and an
    /// unset, missing or unparsable `.lens` file falls back to the picked library
    /// lens. `(None, None)` for a slot of the wrong kind, which cannot happen.
    pub fn flare_inputs(self) -> (Option<&'a Tex>, Option<&'a (u64, String)>) {
        match self {
            AuxSlot::FlareInputs { matte, lens } => (matte, lens),
            _ => (None, None),
        }
    }
}

/// One migrated effect's GPU pass.
///
/// The arguments are what [`crate::fxops::run_ops`] already holds when an op
/// comes round: the engine, the device, the picture the chain is carrying, its
/// size, and whatever the render prepared beside the stack for it. `run` returns
/// the picture after the pass, which for most effects is a new texture.
pub trait GpuEffect: Sync + 'static {
    /// The stable name this answers to — the same `match_name` the effect's
    /// declaration carries in `lumit-core`.
    fn match_name(&self) -> &'static str;

    /// Which parallel input list this effect consumes a slot of.
    /// [`crate::fxops::run_ops`] advances the matching counter exactly as the
    /// old match arms did, so the enumeration in `build.rs` and the consumption
    /// here stay in step (K-387).
    fn aux(&self) -> AuxKind {
        AuxKind::None
    }

    /// Draw the effect, with its parameters read from the resolved bag and its
    /// side-table input (if it declared one) already bound.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex;
}

/// Every migrated effect's GPU pass. Order is irrelevant here (the Add-effect
/// menu reads the catalogue, not this), so it follows the catalogue's for the
/// benefit of anyone reading the two side by side.
static GPU_EFFECTS: &[&dyn GpuEffect] = &[
    &Blur,
    &DirectionalBlur,
    &RadialBlur,
    &Sharpen,
    &SharpenSimple,
    &SpriteFlare,
    &LightWrap,
    &RgbSplit,
    &ChromaticAberration,
    &Flash,
    &ColourBalance,
    &Saturation,
    &Vibrancy,
    &Vignette,
    &Exposure,
    &HueShift,
    &Contrast,
    &Gamma,
    &Temperature,
    &Lut,
    &Dof,
    &Transform,
    &Shake,
    &Glow,
    &BlockGlitch,
    &Scanlines,
    &Datamosh,
    &Echo,
    &MotionBlur,
    &MatteKey,
    &Invert,
    &Tint,
    &LensFlare,
];

/// The GPU pass for `match_name`, or `None` when the effect has no image
/// operation of its own — the orchestration-only case, which is a passthrough
/// rather than a fault (docs/impl/effect-registry.md §3).
///
/// A linear scan of a handful of `&'static str`s, called once per effect per
/// frame and never per pixel — the same shape `fx::schema` has always had.
pub fn gpu_effect(match_name: &str) -> Option<&'static dyn GpuEffect> {
    GPU_EFFECTS
        .iter()
        .copied()
        .find(|g| g.match_name() == match_name)
}

/// Every name this table answers to, for the test that holds it against the
/// catalogue.
pub fn gpu_effect_names() -> impl Iterator<Item = &'static str> {
    GPU_EFFECTS.iter().map(|g| g.match_name())
}

struct Blur;
impl GpuEffect for Blur {
    fn match_name(&self) -> &'static str {
        "blur"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (radius_px, edge, mix) = effects::blur::Blur::read(p).packed();
        fx.blur(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::BlurOp {
                radius_px,
                edge,
                mix,
            },
        )
    }
}

struct DirectionalBlur;
impl GpuEffect for DirectionalBlur {
    fn match_name(&self) -> &'static str {
        "directional_blur"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (length_px, angle_deg, edge, mix) =
            effects::directional_blur::DirectionalBlur::read(p).packed();
        // The unit direction and the tap count are derived here exactly as the
        // old `run_ops` arm derived them, from the same two numbers the CPU
        // reference derives them from.
        let (dx, dy) = lumit_core::fx::rgb_split_offset(1.0, angle_deg);
        fx.dir_blur(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::DirBlurOp {
                dx,
                dy,
                length_px,
                taps: lumit_core::fx::cpu::dir_blur_taps(length_px),
                edge,
                mix,
            },
        )
    }
}

struct RadialBlur;
impl GpuEffect for RadialBlur {
    fn match_name(&self) -> &'static str {
        "radial_blur"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (centre_frac, amount_px, spin, edge, mix) =
            effects::radial_blur::RadialBlur::read(p).packed();
        fx.radial_blur(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::RadialBlurOp {
                centre_frac,
                amount_px,
                taps: lumit_core::fx::cpu::radial_blur_taps(amount_px),
                spin,
                edge,
                mix,
            },
        )
    }
}

struct Sharpen;
impl GpuEffect for Sharpen {
    fn match_name(&self) -> &'static str {
        "sharpen"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, radius_px, threshold, luma_only, mix) =
            effects::sharpen::Sharpen::read(p).packed();
        fx.sharpen(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::SharpenOp {
                amount,
                radius_px,
                threshold,
                luma_only,
                mix,
            },
        )
    }
}

struct SharpenSimple;
impl GpuEffect for SharpenSimple {
    fn match_name(&self) -> &'static str {
        "sharpen_simple"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, radius, mix) = effects::sharpen_simple::SharpenSimple::read(p).packed();
        fx.sharpen_simple(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::SharpenSimpleOp {
                amount,
                radius,
                mix,
            },
        )
    }
}

struct SpriteFlare;
impl GpuEffect for SpriteFlare {
    fn match_name(&self) -> &'static str {
        "sprite_flare"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let s = effects::sprite_flare::SpriteFlare::read(p).packed();
        fx.sprite_flare(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::SpriteFlareOp {
                light: s.light,
                intensity: s.intensity,
                tint: s.tint,
                glow_size: s.glow_size,
                glow_intensity: s.glow_intensity,
                ghosts: s.ghosts,
                ghost_spacing: s.ghost_spacing,
                ghost_size: s.ghost_size,
                ghost_intensity: s.ghost_intensity,
                streak_length: s.streak_length,
                streak_intensity: s.streak_intensity,
                streak_angle_deg: s.streak_angle_deg,
                mix: s.mix,
            },
        )
    }
}

struct LightWrap;
impl GpuEffect for LightWrap {
    fn match_name(&self) -> &'static str {
        "light_wrap"
    }
    /// The Background plate is another layer, rendered alone at this raster —
    /// the layer-input list, off the same counter Depth of field uses because
    /// `build.rs` enumerates both with one predicate (K-358, K-387).
    fn aux(&self) -> AuxKind {
        AuxKind::LayerInput
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (width_px, intensity, mix) = effects::light_wrap::LightWrap::read(p).packed();
        // An absent Background — unset, missing or cyclic — is the passthrough,
        // and so is any neutral setting: the old arm guarded both, and the CPU
        // reference guards the second internally, so the two paths agree about
        // which inputs draw nothing at all.
        match aux.layer_input() {
            Some(background) if width_px > 0.0 && intensity > 0.0 && mix > 0.0 => fx.light_wrap(
                ctx,
                tex,
                w,
                h,
                background,
                &lumit_gpu::fx::LightWrapOp {
                    width_px,
                    intensity,
                    mix,
                },
            ),
            _ => tex.clone(),
        }
    }
}

struct RgbSplit;
impl GpuEffect for RgbSplit {
    fn match_name(&self) -> &'static str {
        "rgb_split"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        // The Wavelength tier runs a different kernel, which is why the effect's
        // `packed` answers with a mode rather than a tuple. The offset vector and
        // the spectral basis are derived here exactly as the old `run_ops` arms
        // derived them, from the same numbers the CPU reference derives them from.
        match effects::rgb_split::RgbSplit::read(p).packed() {
            effects::rgb_split::Split::Classic {
                amount_px,
                angle_deg,
                scale,
                tints,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(amount_px, angle_deg);
                fx.rgb_split(
                    ctx,
                    tex,
                    w,
                    h,
                    &lumit_gpu::fx::RgbSplitOp {
                        dx,
                        dy,
                        scale,
                        tints,
                        mix,
                    },
                )
            }
            effects::rgb_split::Split::Spectral {
                amount_px,
                angle_deg,
                samples,
                tints,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(amount_px, angle_deg);
                let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples, tints);
                fx.spectral_split(
                    ctx,
                    tex,
                    w,
                    h,
                    &lumit_gpu::fx::SpectralSplitOp {
                        dx,
                        dy,
                        amount_px,
                        radial: false,
                        basis,
                        count,
                        mix,
                    },
                )
            }
        }
    }
}

struct ChromaticAberration;
impl GpuEffect for ChromaticAberration {
    fn match_name(&self) -> &'static str {
        "chromatic_aberration"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        match effects::chromatic_aberration::ChromaticAberration::read(p).packed() {
            effects::chromatic_aberration::Fringe::Classic {
                amount_px,
                tints,
                mix,
            } => fx.chromatic_aberration(
                ctx,
                tex,
                w,
                h,
                &lumit_gpu::fx::ChromaticAberrationOp {
                    amount_px,
                    tints,
                    mix,
                },
            ),
            // The radial spectral split (K-144): the old arm passed angle 0.0,
            // so the offset vector is the same `(amount_px, 0)` it always was.
            effects::chromatic_aberration::Fringe::Spectral {
                amount_px,
                samples,
                tints,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(amount_px, 0.0);
                let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples, tints);
                fx.spectral_split(
                    ctx,
                    tex,
                    w,
                    h,
                    &lumit_gpu::fx::SpectralSplitOp {
                        dx,
                        dy,
                        amount_px,
                        radial: true,
                        basis,
                        count,
                        mix,
                    },
                )
            }
        }
    }
}

struct ColourBalance;
impl GpuEffect for ColourBalance {
    fn match_name(&self) -> &'static str {
        "colour_balance"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (lift, gamma, gain, mix) = effects::colour_balance::ColourBalance::read(p).packed();
        fx.colour_balance(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::ColourBalanceOp {
                lift,
                gamma,
                gain,
                mix,
            },
        )
    }
}

struct Saturation;
impl GpuEffect for Saturation {
    fn match_name(&self) -> &'static str {
        "saturation"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (saturation, mix) = effects::saturation::Saturation::read(p).packed();
        fx.saturation(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::SaturationOp { saturation, mix },
        )
    }
}

struct Vibrancy;
impl GpuEffect for Vibrancy {
    fn match_name(&self) -> &'static str {
        "vibrancy"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, mix) = effects::vibrancy::Vibrancy::read(p).packed();
        fx.vibrancy(ctx, tex, w, h, &lumit_gpu::fx::VibrancyOp { amount, mix })
    }
}

struct Flash;
impl GpuEffect for Flash {
    fn match_name(&self) -> &'static str {
        "flash"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        // Strength is the resolve-time envelope (K-385), already in the bag; the
        // wrapper does no time maths of its own, exactly as it does no arithmetic
        // of its own.
        let f = effects::flash::Flash::read(p);
        let (strength, colour, mix) = f.packed(effects::flash::Flash::strength_of(p));
        fx.flash(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::FlashOp {
                strength,
                colour,
                mix,
            },
        )
    }
}

struct Vignette;
impl GpuEffect for Vignette {
    fn match_name(&self) -> &'static str {
        "vignette"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, radius, softness, roundness, ramp, mix) =
            effects::vignette::Vignette::read(p).packed();
        fx.vignette(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::VignetteOp {
                amount,
                radius,
                softness,
                roundness,
                ramp,
                mix,
            },
        )
    }
}

struct Exposure;
impl GpuEffect for Exposure {
    fn match_name(&self) -> &'static str {
        "exposure"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (factor, mix) = effects::exposure::Exposure::read(p).packed();
        fx.exposure(ctx, tex, w, h, &lumit_gpu::fx::ExposureOp { factor, mix })
    }
}

struct HueShift;
impl GpuEffect for HueShift {
    fn match_name(&self) -> &'static str {
        "hue_shift"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (m, mix) = effects::hue_shift::HueShift::read(p).packed();
        fx.hue_shift(ctx, tex, w, h, &lumit_gpu::fx::HueShiftOp { m, mix })
    }
}

struct Contrast;
impl GpuEffect for Contrast {
    fn match_name(&self) -> &'static str {
        "contrast"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (k, mix) = effects::contrast::Contrast::read(p).packed();
        fx.contrast(ctx, tex, w, h, &lumit_gpu::fx::ContrastOp { k, mix })
    }
}

struct Gamma;
impl GpuEffect for Gamma {
    fn match_name(&self) -> &'static str {
        "gamma"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (gamma, mix) = effects::gamma::Gamma::read(p).packed();
        fx.gamma(ctx, tex, w, h, &lumit_gpu::fx::GammaOp { gamma, mix })
    }
}

struct Temperature;
impl GpuEffect for Temperature {
    fn match_name(&self) -> &'static str {
        "temperature"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (gain_r, gain_b, mix) = effects::temperature::Temperature::read(p).packed();
        fx.temperature(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::TemperatureOp {
                gain_r,
                gain_b,
                mix,
            },
        )
    }
}

struct Lut;
impl GpuEffect for Lut {
    fn match_name(&self) -> &'static str {
        "lut"
    }
    /// The k-th `lut` op binds the k-th cube (docs/08 §3.11) — the counter
    /// `run_ops` advances for this kind.
    fn aux(&self) -> AuxKind {
        AuxKind::Lut
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let mix = effects::lut::Lut::read(p).packed();
        // An empty slot — unset, missing, 1D or unreadable file — is the
        // labelled no-op, exactly as the old arm's `if let Some` was; the
        // texture handle is an `Arc`, so passing it back costs nothing.
        match aux.lut() {
            Some(l) => fx.lut(
                ctx,
                tex,
                w,
                h,
                &l.texture,
                l.size,
                mix,
                l.domain_min,
                l.domain_max,
            ),
            None => tex.clone(),
        }
    }
}

struct Dof;
impl GpuEffect for Dof {
    fn match_name(&self) -> &'static str {
        "dof"
    }
    /// The depth pass is the referenced layer rendered alone at this raster —
    /// the k-th consuming op binds the k-th slot (docs/08 §3.22, K-387), the
    /// counter Light wrap shares.
    fn aux(&self) -> AuxKind {
        AuxKind::LayerInput
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // An empty slot — unset, missing or cyclic — is the labelled no-op,
        // exactly as the old arm's `if let Some` was.
        let Some(depth) = aux.layer_input() else {
            return tex.clone();
        };
        // A slot only ever arrives for an op whose Layer row names something
        // (`build.rs`'s `layer_input_param` predicate), so a depth pass being
        // *here* is precisely what "a depth layer is bound" meant in the old
        // resolve arm — the fact the bag cannot carry, since a Layer row never
        // reaches it.
        let d = effects::dof::Dof::read(p).packed(true, effects::dof::Dof::blades_of(p));
        fx.dof(
            ctx,
            tex,
            w,
            h,
            depth,
            &lumit_gpu::fx::DofOp {
                focus: d.focus,
                range: d.range,
                near_aperture: d.near_aperture,
                far_aperture: d.far_aperture,
                blade_normals: d.blade_normals,
                blade_count: d.blade_count,
                apothem2: d.apothem2,
                roundness: d.roundness,
                rim: d.rim,
                aspect_scale: d.aspect_scale,
                threshold: d.threshold,
                bokeh_power: d.bokeh_power,
                repeat_edge: d.repeat_edge,
                depth_bound: true,
                depth_channel: d.depth_channel,
                depth_invert: d.depth_invert,
                use_focus_point: d.use_focus_point,
                focus_point: d.focus_point,
                gamma: d.gamma,
                remove_edge_leak: d.remove_edge_leak,
                detect_edge_threshold: d.detect_edge_threshold,
                display: d.display,
                mix: d.mix,
            },
        )
    }
}

struct Transform;
impl GpuEffect for Transform {
    fn match_name(&self) -> &'static str {
        "transform"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (anchor, position, scale, rotation_deg, opacity, mix) =
            effects::transform::Transform::read(p).packed();
        // The affine is the one lumit-core helper both paths build through, so
        // the CPU reference and the kernel consume identical numbers (K-031).
        let (m, off, opacity) =
            lumit_core::fx::transform_op(anchor, position, scale, rotation_deg, opacity);
        fx.transform(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::TransformOp {
                m,
                off,
                opacity,
                mix,
                // The Transform effect has no Edges control: a transparent
                // border, its long-standing behaviour.
                edge: 0,
            },
        )
    }
}

struct Shake;
impl GpuEffect for Shake {
    fn match_name(&self) -> &'static str {
        "shake"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        use effects::shake::{Shake as ShakeParams, Shaken};
        // Shake dispatches the Transform kernel (docs/08 §3.4: a
        // transform-domain effect — perturb a virtual camera, resample once):
        // the shared affine turns the resolved wobble into the same op the CPU
        // reference builds, so both paths consume bit-identical numbers. With
        // its own motion blur on (T18/K-165) it builds one affine per sub-frame
        // and dispatches the averaging kernel instead, over the same sub-frames
        // `cpu::transform_average` averages. Shake's own Edges control governs
        // the border the wobble reveals, either way.
        let params = ShakeParams::read(p);
        match params.packed(ShakeParams::derived_of(p)) {
            Shaken::Plain { wobble, edge, mix } => {
                let (anchor, position, scale, rot) = lumit_core::fx::shake_affine(
                    w,
                    h,
                    wobble.offset_px,
                    wobble.rotation_deg,
                    wobble.zoom,
                );
                let (m, off, opacity) =
                    lumit_core::fx::transform_op(anchor, position, scale, rot, 1.0);
                fx.transform(
                    ctx,
                    tex,
                    w,
                    h,
                    &lumit_gpu::fx::TransformOp {
                        m,
                        off,
                        opacity,
                        mix,
                        edge,
                    },
                )
            }
            Shaken::Blurred { samples, edge, mix } => {
                let mut taps = [lumit_gpu::fx::ShakeMbTap {
                    m: [1.0, 0.0, 0.0, 1.0],
                    off: [0.0, 0.0],
                }; lumit_gpu::fx::SHAKE_MB_SAMPLES];
                for (t, s) in taps.iter_mut().zip(samples.iter()) {
                    let (anchor, position, scale, rot) =
                        lumit_core::fx::shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
                    let (m, off, _opacity) =
                        lumit_core::fx::transform_op(anchor, position, scale, rot, 1.0);
                    *t = lumit_gpu::fx::ShakeMbTap { m, off };
                }
                fx.shake_mb(
                    ctx,
                    tex,
                    w,
                    h,
                    &lumit_gpu::fx::ShakeMbOp {
                        taps,
                        count: samples.len() as u32,
                        edge,
                        mix,
                    },
                )
            }
        }
    }
}

struct Glow;
impl GpuEffect for Glow {
    fn match_name(&self) -> &'static str {
        "glow"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (radius_px, threshold, knee, intensity, tint, mix) =
            effects::glow::Glow::read(p).packed();
        fx.glow(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::GlowOp {
                radius_px,
                threshold,
                knee,
                intensity,
                tint,
                mix,
            },
        )
    }
}

struct BlockGlitch;
impl GpuEffect for BlockGlitch {
    fn match_name(&self) -> &'static str {
        "block_glitch"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        // The tick is the resolve-time discretised layer time (K-385), already
        // in the bag; the wrapper does no time maths of its own, exactly as it
        // does no arithmetic of its own.
        let b = effects::block_glitch::BlockGlitch::read(p);
        let (
            intensity,
            seed,
            tick,
            block_size_px,
            jitter_frac,
            amount_px,
            chan_px,
            slice_frac,
            mix,
        ) = b.packed(effects::block_glitch::BlockGlitch::tick_of(p));
        fx.block_glitch(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::BlockGlitchOp {
                intensity,
                seed,
                tick,
                block_size_px,
                jitter_frac,
                amount_px,
                chan_px,
                slice_frac,
                mix,
            },
        )
    }
}

struct Scanlines;
impl GpuEffect for Scanlines {
    fn match_name(&self) -> &'static str {
        "scanlines"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        // Intensity carries an old project's folded Darkness and the roll is
        // this frame's offset — both resolve-time derivations (K-385), already
        // in the bag.
        let (i, r) = effects::scanlines::Scanlines::derived_of(p);
        let (intensity, period_px, roll_px, interlace, mix) =
            effects::scanlines::Scanlines::read(p).packed(i, r);
        fx.scanlines(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::ScanlinesOp {
                intensity,
                period_px,
                roll_px,
                interlace,
                mix,
            },
        )
    }
}

struct Datamosh;
impl GpuEffect for Datamosh {
    fn match_name(&self) -> &'static str {
        "datamosh"
    }
    /// Datamosh reads the layer's decoded motion — the current→previous flow
    /// field *and* the −1 neighbour it drags along it. Whole lists, so no
    /// counter advances for this kind.
    fn aux(&self) -> AuxKind {
        AuxKind::FlowField
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // Either input missing — a non-footage layer, or a dropped decode — is a
        // passthrough, never a fault, exactly as the old arm's tuple `if let`
        // was.
        let (Some(flow), Some((_, prev))) = (
            aux.flow_field(),
            aux.neighbours().iter().find(|(o, _)| *o == -1),
        ) else {
            return tex.clone();
        };
        let (ramp, reach) = effects::datamosh::Datamosh::derived_of(p);
        let (intensity, displacement, bloom, steps, mix) =
            effects::datamosh::Datamosh::read(p).packed(ramp, reach);
        fx.datamosh(
            ctx,
            tex,
            prev,
            flow,
            w,
            h,
            &lumit_gpu::fx::DatamoshOp {
                // The blend maths take a single fraction; Mix folds into
                // Intensity here rather than adding a second uniform, since
                // mixing the same two inputs twice collapses to one mix by the
                // product.
                intensity: intensity * mix,
                displacement,
                bloom,
                steps,
            },
        )
    }
}

struct Echo;
impl GpuEffect for Echo {
    fn match_name(&self) -> &'static str {
        "echo"
    }
    /// Echo reads the layer's decoded neighbour frames — the **whole** list, not
    /// a slot of it, so no counter advances for this kind. The render decoded
    /// exactly the offsets the effect's declared temporal window asked for.
    fn aux(&self) -> AuxKind {
        AuxKind::Neighbours
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (weights, mode, mix) = effects::echo::Echo::read(p).packed();
        // The kernel is called whether or not any neighbour arrived, exactly as
        // the old arm called it: an offset with no frame simply contributes
        // nothing, so a layer at its first frame trails off rather than
        // flickering between an echoed and an un-echoed picture.
        let by_offset: Vec<(i32, &Tex)> = aux.neighbours().iter().map(|(o, t)| (*o, t)).collect();
        fx.echo(
            ctx,
            tex,
            &by_offset,
            w,
            h,
            &lumit_gpu::fx::EchoOp { weights, mode, mix },
        )
    }
}

struct MotionBlur;
impl GpuEffect for MotionBlur {
    fn match_name(&self) -> &'static str {
        "motion_blur"
    }
    /// The streak follows the layer's dense motion field (with a confidence
    /// channel, FX-19), which the decode worker computed from the current and
    /// next source frames. A whole texture, so no counter advances for it.
    fn aux(&self) -> AuxKind {
        AuxKind::FlowField
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // With no field (a plain layer, or a decode that dropped the neighbour)
        // it is a passthrough — never a fault.
        let Some(flow) = aux.flow_field() else {
            return tex.clone();
        };
        let (shutter_frac, samples, mix, view) = effects::motion_blur::MotionBlur::read(p).packed();
        fx.motion_blur(
            ctx,
            tex,
            flow,
            w,
            h,
            &lumit_gpu::fx::MotionBlurOp {
                shutter_frac,
                samples,
                mix,
                view: view.code(),
            },
        )
    }
}

struct MatteKey;
impl GpuEffect for MatteKey {
    fn match_name(&self) -> &'static str {
        "matte_key"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let k = effects::matte_key::MatteKey::read(p).packed();
        fx.matte_key(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::MatteKeyOp {
                view: k.view,
                key: k.key,
                gain: k.gain,
                balance: k.balance,
                despill_bias: k.despill_bias,
                alpha_bias: k.alpha_bias,
                spill: k.spill,
                clip_black: k.clip_black,
                clip_white: k.clip_white,
                clip_rollback: k.clip_rollback,
                replace_method: k.replace_method,
                replace_colour: k.replace_colour,
                mix: k.mix,
            },
        )
    }
}

struct Invert;
impl GpuEffect for Invert {
    fn match_name(&self) -> &'static str {
        "invert"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let mix = effects::invert::Invert::read(p).packed();
        fx.invert(ctx, tex, w, h, &lumit_gpu::fx::InvertOp { mix })
    }
}

struct Tint;
impl GpuEffect for Tint {
    fn match_name(&self) -> &'static str {
        "tint"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (black, white, mix) = effects::tint::Tint::read(p).packed();
        fx.tint(ctx, tex, w, h, &lumit_gpu::fx::TintOp { black, white, mix })
    }
}

struct LensFlare;
impl GpuEffect for LensFlare {
    fn match_name(&self) -> &'static str {
        "lens_flare"
    }
    /// The Matte source and the custom `.lens` prescription, counted together off
    /// one index (K-257, K-264, K-387): `build.rs` enumerates the layer's enabled
    /// `lens_flare` effects once and fills both lists in that order, so the k-th
    /// flare op binds the k-th entry of each.
    fn aux(&self) -> AuxKind {
        AuxKind::FlareInputs
    }
    /// The one wrapper that is not thin, and could not be: the flare needs a
    /// **bake** — the prescription parsed, every ghost path ray-probed and ranked,
    /// the starburst transformed — which is far too heavy to redo per frame, so it
    /// is handed over as a closure the GPU side runs only when its parameter-hash
    /// cache misses, and may run beside the frame rather than inside it
    /// (`FxEngine::set_deferred_flare_bakes`, K-350).
    ///
    /// Everything below the bake is still the registry's rule: no arithmetic of
    /// its own. Every frame-time number comes out of the one `lumit-core` module
    /// that owns the formulas (K-031: the CPU reference and the kernels read
    /// identical values), through the effect's `packed`.
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::lens_flare as lf;
        let (matte, custom) = aux.flare_inputs();
        let (lights, light_count) = effects::lens_flare::LensFlare::lights_of(p);
        let params = effects::lens_flare::LensFlare::read(p).packed(lights, light_count);
        let p = &params;

        let (tier_base, tier_lambda, flare_div) = lf::quality_ladder(p.quality);
        // The Detail dial scales the tier's base and wavelength count (K-265) —
        // through the shared helpers, so this equals the CPU reference.
        let grid = lf::detail_base(tier_base, p.detail);
        let lambda_count = lf::detail_lambda(tier_lambda, p.detail);
        let energy = p.ghost_intensity;
        // The traced bands with their eight radiometric sub-samples (K-364),
        // Ghost intensity folded into every sub-weight — the bake's
        // auto-exposure gain joins it GPU-side.
        let bands: Vec<lumit_gpu::fx::FlareBand> = lf::spectral_bands(lambda_count, p.dispersion)
            .into_iter()
            .map(|b| lumit_gpu::fx::FlareBand {
                traced_nm: b.traced_nm,
                sub_idx: b.sub_idx,
                sub_rgb: b
                    .sub_rgb
                    .map(|c| [c[0] * energy, c[1] * energy, c[2] * energy]),
            })
            .collect();
        let op = lumit_gpu::fx::LensFlareOp {
            // Raster pixels → fraction here, where the raster is known (K-260:
            // the parameter is px@comp).
            light_frac: [p.light[0] / w.max(1) as f32, p.light[1] / h.max(1) as f32],
            // Manual mode's lights: ONE entry per light, size and all (K-367). An
            // area source is no longer replicated into point samples — every ray
            // integrates the extent itself, so the extent travels with the light.
            manual_lights: lf::manual_light(p, w, h)
                .iter()
                .map(|l| {
                    [
                        l.pos[0],
                        l.pos[1],
                        l.rgb[0],
                        l.rgb[1],
                        l.rgb[2],
                        l.extent[0],
                        l.extent[1],
                    ]
                })
                .collect(),
            intensity: p.intensity,
            bands,
            max_ghosts: p.max_ghosts,
            coating: p.coating,
            focus_m: p.focus_m,
            fstop: p.fstop,
            blades: p.blades,
            aperture_rotation_deg: p.aperture_rotation_deg,
            roundness: p.roundness,
            aperture_softness: p.aperture_softness,
            ghost_softness: p.ghost_softness,
            grid,
            flare_div,
            screen_transform: lf::screen_transform(w),
            starburst_intensity: p.starburst_intensity,
            scale: p.scale,
            anamorphic: p.anamorphic,
            source: p.source,
            threshold: p.threshold,
            threshold_softness: p.threshold_softness,
            light_tint: p.light_tint,
            use_source_colour: p.use_source_colour,
            blend: p.blend,
            mix: p.mix,
            bake_key: lf::bake_key_with(p, custom.map(|(h, _)| *h)),
        };
        let custom_text = custom.map(|(_, text)| text.clone());
        // Manual mode's frame-time grid probe (K-267): the GPU hands back its
        // cached bake's tables and this closure runs the one lumit-core probe
        // both twins share, at the frame's actual light direction.
        let light_frac = op.light_frac;
        let aspect = h as f32 / w.max(1) as f32;
        let probe = move |pb: &lumit_gpu::fx::FlareProbeBake| {
            let needs = lf::frame_grid_needs_from_rows(
                pb.surfaces,
                pb.ghosts,
                pb.sensor_z_mm,
                pb.focal_mm,
                pb.pupil_mm,
                pb.start_z_mm,
                pb.pair_count,
                lf::light_direction(light_frac, aspect, pb.focal_mm),
                params.coating,
                lf::fstop_scale(pb.native_fstop, params.fstop),
                lf::focus_shift_mm(params.focus_m, pb.focal_mm),
            );
            lf::plan_frame_grids(grid, pb.spreads, &needs)
        };
        fx.lens_flare(
            ctx,
            tex,
            w,
            h,
            &op,
            matte,
            // The bake as something the bake thread can own and run (K-350): one
            // small `Arc` a flare a frame, beside a pass that traces hundreds of
            // thousands of rays. Whether it is actually run beside the frame or
            // inside it is the engine's policy, not this call's — see
            // `FxEngine::set_deferred_flare_bakes`.
            &(std::sync::Arc::new(move || {
                let b = lf::bake_with(&params, custom_text.as_deref());
                lumit_gpu::fx::FlareBakeData {
                    surfaces: b
                        .surfaces
                        .iter()
                        .map(|s| {
                            [
                                s.radius_mm,
                                s.z_mm,
                                s.semi_ap_mm,
                                s.cauchy_a,
                                s.cauchy_b,
                                s.coating_layers,
                                s.is_stop,
                                0.0,
                            ]
                        })
                        .collect(),
                    ghosts: b.pairs.clone(),
                    spreads: b.spreads.clone(),
                    sensor_z_mm: b.sensor_z_mm,
                    focal_mm: b.focal_mm,
                    native_fstop: b.native_fstop,
                    pupil_mm: b.pupil_mm,
                    start_z_mm: b.start_z_mm,
                    energy_gain: b.energy_gain,
                    reflectance: b.reflectance.clone(),
                    starburst: b.starburst,
                    sb_res: lf::STARBURST_RES,
                    sb_fields: lf::STARBURST_FIELDS as u32,
                }
            }) as lumit_gpu::fx::FlareBake),
            &probe,
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::fx::BUILTIN_DEFS;

    /// The two registries must agree, and nothing but a test can make them:
    /// they are joined by a string. Every migrated effect with an image
    /// operation needs exactly one GPU pass, and every GPU pass must name an
    /// effect that exists (docs/impl/effect-registry.md §7, test 3).
    #[test]
    fn every_migrated_effect_has_a_gpu_entry() {
        for def in BUILTIN_DEFS.iter() {
            let name = def.schema().match_name;
            if def.is_image_op() {
                assert!(
                    gpu_effect(name).is_some(),
                    "{name} is migrated and draws pixels, but has no GPU pass"
                );
            } else {
                assert!(
                    gpu_effect(name).is_none(),
                    "{name} is orchestration-only and must not have a GPU pass"
                );
            }
        }
        for name in gpu_effect_names() {
            assert!(
                BUILTIN_DEFS.get(name).is_some(),
                "the GPU table names {name}, which no effect declares"
            );
        }
    }

    /// An effect whose real input is a file or another layer must say which
    /// list that input arrives on (K-387), because `resolve_into_arena`
    /// deliberately drops those parameter kinds: only the render knows which
    /// cube loaded or which layer was rendered, so nothing reaches the bag.
    ///
    /// This is the gate that makes the silence safe, and it lives here because
    /// it is the only place both halves are visible — the declaration in
    /// `lumit-core`, the consumption in this table. Without it, migrating an
    /// effect and forgetting its `aux()` is a picture that renders perfectly and
    /// quietly ignores its grade.
    #[test]
    fn a_side_table_effect_declares_the_list_it_consumes() {
        use lumit_core::fx::ParamKind;
        for def in BUILTIN_DEFS.iter() {
            let name = def.schema().match_name;
            let side_input = def
                .schema()
                .params
                .iter()
                .any(|p| matches!(p.kind, ParamKind::File { .. } | ParamKind::Layer { .. }));
            if !side_input {
                continue;
            }
            let gpu = gpu_effect(name).unwrap_or_else(|| {
                panic!("{name} takes a file or layer input but has no GPU pass to receive it")
            });
            assert_ne!(
                gpu.aux(),
                AuxKind::None,
                "{name} declares a file or layer row, but its GPU pass claims no list — \
                 the input it was given would never arrive"
            );
        }
    }

    /// One name, one pass. Two wrappers answering to the same string would make
    /// which kernel runs depend on the order of this file.
    #[test]
    fn no_two_gpu_passes_share_a_name() {
        let mut seen: Vec<&str> = Vec::new();
        for name in gpu_effect_names() {
            assert!(!seen.contains(&name), "two GPU passes answer to {name}");
            seen.push(name);
        }
    }

    /// The whole path, end to end: a real effect instance resolves into the
    /// arena, `run_ops` finds this table by the effect's name, and the kernel
    /// draws what the CPU reference draws.
    ///
    /// This is the one link no compiler checks. Every other failure mode here is
    /// silent and looks like a picture: a lookup that misses leaves the texture
    /// untouched, and a bag read wrongly (250 where 2.5 was meant) still
    /// renders — just not the right thing. So the test pins both ends: the
    /// output must have *moved* from the input, and it must land where the CPU
    /// reference lands. The fp16 tolerance is the oracles' business
    /// (`wgsl_saturation_matches_the_cpu_oracle`); this only asks whether the
    /// right numbers reached the right kernel.
    #[test]
    fn a_migrated_effect_renders_through_run_ops() {
        let Ok(ctx) = GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (8u32, 8u32);
        let source: Vec<f32> = (0..(w * h * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect();

        // A heavily desaturating instance, so a passthrough cannot pass for a
        // render: Saturation 20 % visibly greys the corpus.
        let mut inst = lumit_core::fx::instantiate("saturation").expect("saturation is a built-in");
        for p in &mut inst.params {
            if p.id == "saturation" {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(20.0));
            }
        }
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            None,
            &[],
            &[],
            &[],
            &[],
            None,
        );
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        let mut cpu = source.clone();
        lumit_core::fx::cpu::apply_stack(&mut cpu, w, h, &ops);

        assert_ne!(
            gpu, source,
            "the op passed the texture through — the GPU table was never reached"
        );
        for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
            assert!(
                (g - c).abs() < 1e-2,
                "pixel {i}: GPU {g} vs CPU reference {c} — the bag reached the \
                 kernel with the wrong numbers"
            );
        }
    }

    /// **Shake picks its own kernel** (docs/08 §3.4, T18/K-165, K-388).
    ///
    /// Shake is the one migrated effect whose dispatch forks: plain, it is the
    /// Transform kernel; with its own motion blur on, it is the averaging one,
    /// fed nine affines. Nothing but this test joins the fork to the bag — a
    /// wrapper that read the sub-frames and still called `transform` would
    /// render a picture, just not a smeared one — so both modes run end to end
    /// against the CPU reference, and the two must differ from each other.
    #[test]
    fn shake_renders_through_run_ops_in_both_modes() {
        let Ok(ctx) = GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (16u32, 16u32);
        let source: Vec<f32> = (0..(w * h * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect();

        // A big wobble, so a passthrough cannot pass for a render: 8 % of this
        // raster's diagonal is a shift of a pixel or two, with a twist and a
        // depth pump on top.
        let shaken = |motion_blur: bool| {
            let mut inst = lumit_core::fx::instantiate("shake").expect("shake is a built-in");
            for p in &mut inst.params {
                let v = match p.id.as_str() {
                    "amplitude" => 8.0,
                    "rotation" => 6.0,
                    "z_amp" => 5.0,
                    "mb_amount" => 0.9,
                    "motion_blur" => {
                        p.value = lumit_core::model::EffectValue::Bool(motion_blur);
                        continue;
                    }
                    _ => continue,
                };
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(v));
            }
            lumit_core::fx::resolve_stack(
                std::slice::from_ref(&inst),
                0.4,
                ((w * w + h * h) as f32).sqrt(),
                1.0,
                &lumit_core::fx::MarkerContext::NONE,
                std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
            )
        };

        let rendered = |ops: &lumit_core::fx::ResolvedStack| {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = crate::fxops::run_ops(
                &fx,
                &ctx,
                tex,
                w,
                h,
                ops,
                &[],
                None,
                &[],
                &[],
                &[],
                &[],
                None,
            );
            let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");
            let mut cpu = source.clone();
            lumit_core::fx::cpu::apply_stack(&mut cpu, w, h, ops);
            (gpu, cpu)
        };

        let (plain_gpu, plain_cpu) = rendered(&shaken(false));
        let (smeared_gpu, smeared_cpu) = rendered(&shaken(true));
        for (name, gpu, cpu) in [
            ("plain", &plain_gpu, &plain_cpu),
            ("smeared", &smeared_gpu, &smeared_cpu),
        ] {
            assert_ne!(
                gpu, &source,
                "{name}: the op passed the texture through — the GPU table was never reached"
            );
            for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
                assert!(
                    (g - c).abs() < 1e-2,
                    "{name} pixel {i}: GPU {g} vs CPU reference {c} — the bag reached the                      kernel with the wrong numbers"
                );
            }
        }
        assert_ne!(
            plain_gpu, smeared_gpu,
            "the motion-blur toggle must pick the other kernel"
        );
    }

    /// **The k-th LUT op binds the k-th cube** (docs/08 §3.11, K-387).
    ///
    /// The whole threading contract in one picture. `build.rs` enumerates a
    /// layer's enabled `lut` effects in stack order, and `run_ops` walks a
    /// counter down the ops in the same order; nothing but the counting joins
    /// the two, so a slot that is skipped or double-counted grades the wrong
    /// effect — a project where dragging one LUT above another moves the grade
    /// to a layer nobody touched.
    ///
    /// The failure this pins is the tempting one: advancing the counter only
    /// when a cube is actually there. The first slot here is deliberately
    /// **empty** (an unset or unreadable file — the passthrough every list
    /// allows), so an implementation that skips it hands the second op the first
    /// slot and renders no grade at all.
    #[test]
    fn the_kth_lut_op_binds_the_kth_slot() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (4u32, 4u32);
        // Opaque yellow: full red and green, no blue. Every channel sits on a
        // corner of the two-entry cube below, so the answer is the cube's own
        // value rather than an interpolation of it.
        let source: Vec<f32> = (0..(w * h)).flat_map(|_| [1.0f32, 1.0, 0.0, 1.0]).collect();

        // A grade that takes green to zero and leaves red and blue alone.
        let cube: Vec<[f32; 3]> = (0..8u32)
            .map(|i| [(i & 1) as f32, 0.0, ((i >> 2) & 1) as f32])
            .collect();
        let kill_green = crate::fxops::LoadedLut {
            texture: lumit_gpu::fx::upload_lut_3d(&ctx, 2, &cube),
            size: 2,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
        };

        let inst = lumit_core::fx::instantiate("lut").expect("lut is a built-in");
        let ops = lumit_core::fx::resolve_stack(
            &[inst.clone(), inst],
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 2, "two LUT ops, two slots to bind");

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            None,
            // Slot 0 empty, slot 1 the grade: only the *second* op grades.
            &[None, Some(kill_green)],
            &[],
            &[],
            &[],
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        assert!(
            got[1] < 1e-2,
            "green is {} — the second op did not bind the second slot",
            got[1]
        );
        assert!(
            (got[0] - 1.0).abs() < 1e-2 && got[2] < 1e-2,
            "the grade changed a channel it was told to leave alone: {got:?}"
        );
    }

    /// **Depth of field and Light wrap count along ONE layer-input list**
    /// (docs/impl/layer-input.md §2, K-358, K-387).
    ///
    /// `build.rs` fills a slot for every enabled effect that declares a Layer
    /// row — one predicate, `layer_input_param`, covering both — and `run_ops`
    /// walks a single counter down the ops in the same order. Two counters, or a
    /// counter that only advanced when a slot was actually filled, would hand the
    /// second effect the first effect's plate: a project where adding a Depth of
    /// field above a Light wrap silently moves which layer the wrap reads.
    ///
    /// So the first slot here is deliberately **empty** and belongs to the Depth
    /// of field — the passthrough case, which is where a "skip the counter when
    /// there is nothing to bind" implementation goes wrong — and the second holds
    /// a bright plate for the Light wrap. If the counter is shared and
    /// unconditional, the wrap lights the foreground's edge; if it is not, the
    /// wrap reads the empty slot and draws nothing at all.
    #[test]
    fn depth_of_field_and_light_wrap_share_one_layer_input_counter() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);

        // A dim opaque square in an empty frame: a real matte edge for the wrap
        // to find, and dark enough that a spill is unmistakable.
        let mut fg = vec![0.0f32; (w * h * 4) as usize];
        for y in 6..18u32 {
            for x in 8..24u32 {
                let i = ((y * w + x) * 4) as usize;
                fg[i] = 0.05;
                fg[i + 1] = 0.05;
                fg[i + 2] = 0.05;
                fg[i + 3] = 1.0;
            }
        }
        let plate: Vec<f32> = (0..(w * h) as usize)
            .flat_map(|_| [2.0f32, 2.0, 2.0, 1.0])
            .collect();

        // Stack order: Depth of field (its Depth row unset — an empty slot),
        // then Light wrap with a real Width so it has something to draw.
        let dof = lumit_core::fx::instantiate("dof").expect("dof is a built-in");
        let mut wrap = lumit_core::fx::instantiate("light_wrap").expect("light_wrap is a built-in");
        for p in &mut wrap.params {
            if p.id == "width" {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(5.0));
            }
        }
        let ops = lumit_core::fx::resolve_stack(
            &[dof, wrap],
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 2, "two consuming ops, two slots to bind");

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &fg, w, h);
        let plate_tex = lumit_gpu::fx::upload_linear_f32(&ctx, &plate, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            None,
            &[],
            // Slot 0 empty (the Depth of field's), slot 1 the plate (the wrap's).
            &[
                crate::fxops::LayerInput::Absent,
                crate::fxops::LayerInput::Texture(plate_tex),
            ],
            &[],
            &[],
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        // Just inside the square's left edge: the band the wrap paints.
        let at = |x: u32, y: u32| got[((y * w + x) * 4) as usize];
        assert!(
            at(9, 12) > 0.05 + 1e-2,
            "the edge band is {} — the wrap never saw the second slot",
            at(9, 12)
        );
        // And nowhere outside the matte: an empty pixel stays empty, which is
        // also what proves the Depth of field passed through rather than
        // gathering the plate it was never given.
        for i in (0..got.len()).step_by(4) {
            if fg[i + 3] == 0.0 {
                assert_eq!(
                    got[i + 3],
                    0.0,
                    "pixel {} gained coverage outside the matte",
                    i / 4
                );
            }
        }
    }

    /// **Echo is handed the neighbour frames themselves** (docs/08 §3.13,
    /// K-387). The whole-list kinds take no counter, which makes them look like
    /// the easy case — but an effect that receives an empty list where the
    /// render decoded four frames renders a perfectly ordinary picture with no
    /// trail on it, and nothing else in the pipeline notices. So the trail is
    /// asserted, not the plumbing: a dark frame with a bright neighbour behind
    /// it must come out brighter than it went in.
    #[test]
    fn echo_receives_the_decoded_neighbours() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (4u32, 4u32);
        let source: Vec<f32> = (0..(w * h)).flat_map(|_| [0.2f32, 0.2, 0.2, 1.0]).collect();
        let previous: Vec<f32> = (0..(w * h)).flat_map(|_| [0.8f32, 0.8, 0.8, 1.0]).collect();

        let inst = lumit_core::fx::instantiate("echo").expect("echo is a built-in");
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let neighbours = [(-1, lumit_gpu::fx::upload_linear_f32(&ctx, &previous, w, h))];
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &neighbours,
            None,
            &[],
            &[],
            &[],
            &[],
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        assert!(
            got[0] > 0.2 + 1e-2,
            "red is {} — the neighbour list never reached the kernel",
            got[0]
        );
    }
}
