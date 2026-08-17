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

type Tex = wgpu::Texture;

/// One migrated effect's GPU pass.
///
/// The arguments are what [`crate::fxops::run_ops`] already holds when an op
/// comes round: the engine, the device, the picture the chain is carrying, and
/// its size. `run` returns the picture after the pass, which for most effects is
/// a new texture.
pub trait GpuEffect: Sync + 'static {
    /// The stable name this answers to — the same `match_name` the effect's
    /// declaration carries in `lumit-core`.
    fn match_name(&self) -> &'static str;

    /// Draw the effect, with its parameters read from the resolved bag.
    fn run(&self, fx: &FxEngine, ctx: &GpuContext, tex: &Tex, w: u32, h: u32, p: Params<'_>)
        -> Tex;
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
    &RgbSplit,
    &ChromaticAberration,
    &ColourBalance,
    &Saturation,
    &Vibrancy,
    &Vignette,
    &Exposure,
    &HueShift,
    &Contrast,
    &Gamma,
    &Temperature,
    &Invert,
    &Tint,
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
    ) -> Tex {
        let (amount, mix) = effects::vibrancy::Vibrancy::read(p).packed();
        fx.vibrancy(ctx, tex, w, h, &lumit_gpu::fx::VibrancyOp { amount, mix })
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
    ) -> Tex {
        let (black, white, mix) = effects::tint::Tint::read(p).packed();
        fx.tint(ctx, tex, w, h, &lumit_gpu::fx::TintOp { black, white, mix })
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
}
