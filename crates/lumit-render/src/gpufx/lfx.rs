//! The GPU half of an LFX effect: its passes, run in order on the frame batch.
//!
//! **In plain terms.** `lumit_core::fx::lfx` keeps the table of installed
//! effects and their shaders read and wrapped. This draws them: one dispatch
//! per pass, each pass's picture handed to the passes after it, the last one
//! mixed against the input by the effect's Mix row. Anything that stops an
//! effect drawing (missing, refused or will not compile) leaves
//! the picture as it came in, and [`status`] says which.

use std::sync::Mutex;

use lumit_core::fx::effects::custom_shader as cs;
use lumit_core::fx::lfx::{self, LfxEffect, LfxError};
use lumit_core::fx::{EffectDef, Params};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use super::{AuxSlot, CustomShader, GpuEffect};

type Tex = wgpu::Texture;

/// Install one effect and register it in both tables: its pass here first,
/// then its def in the catalogue, so no catalogue entry is ever without its
/// pass. On a rescan the def and pass are already there and only the shader
/// is replaced. What the extension scan calls for every effect it finds.
///
/// # Errors
/// What [`lfx::install`] refuses.
pub fn register(effect: LfxEffect) -> Result<&'static dyn EffectDef, LfxError> {
    let def = lfx::install(effect)?;
    let name = def.schema().match_name;
    if super::gpu_effect(name).is_none() {
        super::register_gpu_effect(Box::leak(Box::new(LfxPass(name))));
    }
    lumit_core::fx::BUILTIN_DEFS.register(def);
    Ok(def)
}

/// Whether an LFX effect draws, and why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfxStatus {
    Draws,
    /// The project names it and no extension here carries it.
    Missing {
        extension: String,
    },
    /// The shader breaks the contract; the sentence says how.
    Refused(String),
    /// The shader does not compile; the compiler's message, on the
    /// shader's own lines.
    WontCompile(String),
}

/// What the panel shows beside an LFX effect, as `shader_status` is for the
/// Custom shader. Compiles once per distinct shader.
#[must_use]
pub fn status(match_name: &str) -> LfxStatus {
    let Some(entry) = lfx::installed(match_name) else {
        return LfxStatus::Missing {
            extension: lfx::extension_of(match_name)
                .unwrap_or(match_name)
                .to_owned(),
        };
    };
    let program = match &entry.program {
        Ok(program) => *program,
        Err(refusal) => return LfxStatus::Refused(refusal.to_string()),
    };
    match compiled(entry.key_hash, program) {
        Some(why) => LfxStatus::WontCompile(why),
        None => LfxStatus::Draws,
    }
}

/// The compiler's word on every pass, kept per key hash.
fn compiled(key: u64, program: &lumit_core::fx::shader::ShaderProgram) -> Option<String> {
    static SEEN: Mutex<Vec<(u64, Option<String>)>> = Mutex::new(Vec::new());
    if let Some((_, why)) = SEEN.lock().ok()?.iter().find(|(k, _)| *k == key) {
        return why.clone();
    }
    let why = program
        .passes
        .iter()
        .find_map(|(_, text)| lumit_gpu::fx::validate(text).err())
        .map(|message| program.remap_error(&message));
    if let Ok(mut seen) = SEEN.lock() {
        seen.push((key, why.clone()));
    }
    why
}

/// One installed effect's pass, by match name.
struct LfxPass(&'static str);

impl GpuEffect for LfxPass {
    fn match_name(&self) -> &'static str {
        self.0
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
        // Missing at resolve (no hash), a lost device or a refused shader:
        // the input unchanged, never a fault.
        if cs::hash_in(p).is_none() || ctx.device_lost() {
            return tex.clone();
        }
        let Some(entry) = lfx::installed(self.0) else {
            return tex.clone();
        };
        let Ok(program) = entry.program.as_ref().map(|p| *p) else {
            return tex.clone();
        };
        // Every pipeline before any dispatch, so a pass that will not compile
        // costs nothing.
        let mut pipelines = Vec::with_capacity(program.passes.len());
        for (key, text) in &program.passes {
            match fx.lfx_pipeline(ctx, *key, text) {
                Ok(pipeline) => pipelines.push(pipeline),
                Err(_) => return tex.clone(),
            }
        }
        let (instance, lt) = aux.op();
        // The instance's own seed, as the Custom shader's.
        let seed = (instance.as_u128() as u64 ^ (instance.as_u128() >> 64) as u64) as u32;
        let input2 = aux.layer_input();
        let [fx0, fy0, fw, fh] =
            lumit_core::fx::frame_rect(p).unwrap_or([0.0, 0.0, w as f32, h as f32]);
        let mix = (p.float(lumit_core::fx::MIX_ID, 100.0) / 100.0).clamp(0.0, 1.0);
        let params = program.pack(p);
        let passes = entry.passes();
        let mut done: Vec<Tex> = Vec::with_capacity(passes.len());
        for (i, ((_, scale), pipeline)) in passes.iter().zip(&pipelines).enumerate() {
            let last = i + 1 == passes.len();
            let size = |n: u32| ((n as f32 * scale).round() as u32).max(1);
            // Passes before the last are kept as they come; the last is the
            // effect's output, mixed by its Mix row.
            let header = CustomShader::header(
                [fx0 as u32, fy0 as u32],
                [fw as u32, fh as u32],
                [w, h],
                p.float(cs::COMP_SCALE, 1.0),
                lt,
                seed,
                if last { mix } else { 1.0 },
                false,
                input2.is_some(),
            );
            let before: Vec<Option<&Tex>> = done.iter().map(Some).collect();
            let out = fx.lfx_pass(
                ctx,
                pipeline,
                tex,
                None,
                input2,
                &before,
                size(w),
                size(h),
                &header,
                &params,
            );
            done.push(out);
        }
        done.pop().unwrap_or_else(|| tex.clone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const W: u32 = 8;
    const H: u32 = 8;

    /// A first pass at half size that writes its own uv and the input's
    /// blue, and a last pass that reads it back at full size.
    const TWO_PASS: &str = r"
fn half(uv: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(uv.x, uv.y, lumit_sample(uv).b, 1.0);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let h = lumit_read(pass_half, uv);
    return vec4<f32>(h.rgb, lumit_sample(uv).a);
}
";

    fn effect(id: &str, shader: &str, passes: &[(&str, f32)]) -> LfxEffect {
        LfxEffect {
            extension: "test".into(),
            id: id.into(),
            label: "Two pass".into(),
            category: "Stylise".into(),
            version: 1,
            api: 1,
            passes: passes
                .iter()
                .map(|(name, scale)| lfx::LfxPass {
                    name: (*name).into(),
                    scale: *scale,
                })
                .collect(),
            input: None,
            shader: shader.into(),
            interface: false,
        }
    }

    /// The shader's own bilinear read, on the CPU: `tw` by `th` texels of
    /// RGBA, read at uv the way `lumit_bilinear` reads a texture of that size.
    fn bilinear(t: &[f32], tw: u32, th: u32, uv: [f32; 2]) -> [f32; 4] {
        let f = [uv[0] * tw as f32 - 0.5, uv[1] * th as f32 - 0.5];
        let base = [f[0].floor(), f[1].floor()];
        let frac = [f[0] - base[0], f[1] - base[1]];
        let clamp = |v: f32, n: u32| (v as i32).clamp(0, n as i32 - 1) as u32;
        let (x0, x1) = (clamp(base[0], tw), clamp(base[0] + 1.0, tw));
        let (y0, y1) = (clamp(base[1], th), clamp(base[1] + 1.0, th));
        let at = |x: u32, y: u32, c: usize| t[((y * tw + x) * 4) as usize + c];
        let mut out = [0.0; 4];
        for (c, slot) in out.iter_mut().enumerate() {
            let top = at(x0, y0, c) + (at(x1, y0, c) - at(x0, y0, c)) * frac[0];
            let bottom = at(x0, y1, c) + (at(x1, y1, c) - at(x0, y1, c)) * frac[0];
            *slot = top + (bottom - top) * frac[1];
        }
        out
    }

    /// Two passes, the first at half size, against the same arithmetic done
    /// on the CPU. Guards wrong pixels.
    #[test]
    fn a_two_pass_effect_draws_what_its_passes_say() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let def = register(effect(
            "two_pass",
            TWO_PASS,
            &[("half", 0.5), ("shade", 1.0)],
        ))
        .expect("it installs");
        assert_eq!(status(def.schema().match_name), LfxStatus::Draws);

        let src: Vec<f32> = (0..(W * H * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect();
        let inst = lumit_core::fx::instantiate(def.schema().match_name).expect("instantiates");
        assert_eq!(
            inst.effect.namespace,
            lumit_core::model::EffectNamespace::Lfx
        );
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        let out = crate::fxops::run_ops(
            fx,
            &ctx,
            lumit_gpu::fx::upload_linear_f32(&ctx, &src, W, H),
            W,
            H,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &crate::fxops::Side::NONE,
            &[],
            None,
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, W, H).expect("readback");

        // The half-size pass: 4 by 4, each texel its own uv and the input's
        // blue there.
        let (hw, hh) = (W / 2, H / 2);
        let mut half = Vec::new();
        for y in 0..hh {
            for x in 0..hw {
                let uv = [(x as f32 + 0.5) / hw as f32, (y as f32 + 0.5) / hh as f32];
                let c = bilinear(&src, W, H, uv);
                half.extend_from_slice(&[uv[0], uv[1], c[2], 1.0]);
            }
        }
        for y in 0..H {
            for x in 0..W {
                let uv = [(x as f32 + 0.5) / W as f32, (y as f32 + 0.5) / H as f32];
                let want = bilinear(&half, hw, hh, uv);
                let i = ((y * W + x) * 4) as usize;
                for c in 0..3 {
                    assert!(
                        (got[i + c] - want[c]).abs() < 3e-3,
                        "pixel {x},{y} channel {c}: {} vs {}",
                        got[i + c],
                        want[c]
                    );
                }
                assert!((got[i + 3] - 1.0).abs() < 1e-3, "alpha is the input's");
            }
        }
    }
}
