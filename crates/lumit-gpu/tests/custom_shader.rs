//! The Custom shader's GPU-side tests (docs/impl/custom-shader.md §8 items 11,
//! 12, 15, 16, 17, 18, 19 and the line-number half of 10, K-650).
//!
//! **In plain terms.** The half of the effect that needs a graphics card, and the
//! half that only needs the shader compiler. The compiler half runs everywhere,
//! including on a machine and a CI runner with no card, which is the whole point
//! of validating a user's text through naga rather than at pipeline creation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::fx::shader;
use lumit_gpu::fx::{readback_linear_f32, upload_linear_f32, validate, FxEngine};
use lumit_gpu::GpuContext;

/// A shader with one of everything the host hands in, so a change to the
/// prologue that broke any binding fails here rather than on somebody's machine.
const EVERYTHING: &str = r#"
struct Params {
    /// @slider(0, 2) @default(1) Gain
    gain: f32,
    /// @colour @default(0, 0, 0, 1) Tint
    tint: vec4<f32>,
    /// @point @default(0, 0) Centre
    centre: vec2<f32>,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let a = lumit_sample(uv);
    let b = lumit_sample2(uv);
    let c = lumit_orig(uv);
    let d = lumit_load(vec2<i32>(0, 0));
    let k = lumit_matte(uv) * lumit.matte_on + lumit.input2_on;
    let t = lumit.time + f32(lumit.seed) + lumit.comp_scale;
    let px = lumit_px(uv) + p.centre;
    let u = lumit_premult(lumit_unpremult(a));
    return (u * p.gain + p.tint + b * 0.0 + c * 0.0 + d * 0.0)
        + vec4<f32>(k * 0.0 + t * 0.0 + px.x * 0.0);
}
"#;

/// Invert, the golden tiny shader: the picture's colour taken from one, its
/// alpha left alone.
const INVERT: &str = r#"
fn shade(uv: vec2<f32>) -> vec4<f32> {
    let c = lumit_unpremult(lumit_sample(uv));
    return lumit_premult(vec4<f32>(1.0 - c.rgb, c.a));
}
"#;

fn program(source: &str) -> &'static shader::ShaderProgram {
    shader::program_for(source).expect("a program")
}

/// The header the host hands in, at full resolution, Mix 100, nothing bound.
fn header(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(shader::HEADER_SIZE);
    for v in [0u32, 0, w, h] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1.0f32.to_le_bytes()); // comp_scale
    out.extend_from_slice(&0.0f32.to_le_bytes()); // time
    out.extend_from_slice(&7u32.to_le_bytes()); // seed
    out.extend_from_slice(&1.0f32.to_le_bytes()); // mix_amt
    out.extend_from_slice(&0.0f32.to_le_bytes()); // matte_on
    out.extend_from_slice(&0.0f32.to_le_bytes()); // input2_on
    out.extend_from_slice(&[0u8; 8]); // the two pads
    assert_eq!(out.len(), shader::HEADER_SIZE);
    out
}

/// A small picture with an alpha edge and an HDR spike.
fn picture(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = (x + y) as f32 / (w + h) as f32;
            let a = if x < w / 2 { 1.0 } else { 0.5 };
            img[i] = g * a;
            img[i + 1] = (1.0 - g) * a;
            img[i + 2] = 0.25 * a;
            img[i + 3] = a;
        }
    }
    img
}

// ----------------------------------------------------------------- §8 item 11

#[test]
fn the_assembled_module_validates() {
    // The host's own wrapper, round every fixture, through the K-263 road — so a
    // change to the prologue or the epilogue cannot ship broken. No graphics
    // card involved.
    for (name, source) in [
        ("everything", EVERYTHING),
        ("invert", INVERT),
        (
            "no parameters at all",
            "fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.5); }",
        ),
    ] {
        let p = program(source);
        if let Err(why) = validate(&p.assembled) {
            panic!("{name} did not validate:\n{why}\n\n{}", p.assembled);
        }
    }
}

#[test]
fn a_broken_shader_refuses_calmly_with_the_users_own_line_number() {
    // A source whose third line will not compile, behind forty-odd lines of
    // host prologue. What the person sees must say three.
    let p = program(
        "fn shade(uv: vec2<f32>) -> vec4<f32> {\n\
         \x20   let a = 1.0;\n\
         \x20   return nonsense_function(a);\n\
         }\n",
    );
    let raw = validate(&p.assembled).expect_err("a call to nothing must not validate");
    assert!(
        raw.contains(&format!("wgsl:{}", p.prologue_lines + 3)),
        "naga counts from the top of the assembled module: {raw}"
    );
    let shown = p.remap_error(&raw);
    assert!(
        shown.contains("wgsl:3:"),
        "the user is shown their own line: {shown}"
    );
    assert!(
        !shown.starts_with("in the host's own wrapper"),
        "and this one is theirs, not ours: {shown}"
    );
    // It is a message, not a fault: nothing here panics, and the effect renders
    // its input unchanged.
    assert!(!shown.is_empty());
}

#[test]
fn an_error_in_the_hosts_own_wrapper_says_so() {
    let p = program(INVERT);
    let mistake = p.remap_error("error: something\n  ┌─ wgsl:2:5\n");
    assert!(
        mistake.starts_with("in the host's own wrapper"),
        "a line inside the prologue is a bug in Lumit and reads like one: {mistake}"
    );
}

// ------------------------------------------------- §8 items 12, 15, 16, 17, 18

#[test]
fn a_golden_shader_renders_deterministically() {
    let Ok(ctx) = GpuContext::headless() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (16u32, 12u32);
    let img = picture(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let p = program(INVERT);
    let (pipeline, badge) = fx
        .shader_pipeline(&ctx, 1, p.source_hash, &p.assembled)
        .expect("invert compiles");
    assert!(badge.is_none(), "a shader that compiles wears no badge");
    let params = p.pack(lumit_core::fx::Params::new(&[]));
    let draw = || {
        let out = fx.custom_shader(
            &ctx,
            &pipeline,
            &tex,
            &tex,
            None,
            None,
            w,
            h,
            &header(w, h),
            &params,
        );
        readback_linear_f32(&ctx, &out, w, h).expect("readback")
    };
    let first = draw();
    let second = draw();
    assert_eq!(
        first, second,
        "the same inputs render bit-identically twice (K-031)"
    );
    // And it is Invert: unpremultiplied colour taken from one, alpha untouched.
    for i in (0..first.len()).step_by(4) {
        let a = img[i + 3];
        let want = if a > 0.0 { (1.0 - img[i] / a) * a } else { 0.0 };
        assert!(
            (first[i] - want).abs() < 2e-3,
            "pixel {i}: {} vs {want}",
            first[i]
        );
        assert!((first[i + 3] - a).abs() < 2e-3, "alpha is left alone");
    }
}

#[test]
fn a_nan_returned_by_a_shader_never_leaves_the_effect() {
    let Ok(ctx) = GpuContext::headless() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (8u32, 8u32);
    let tex = upload_linear_f32(&ctx, &picture(w, h), w, h);
    // Nought over nought and one over nought, in the user's own arithmetic.
    let p = program(
        "fn shade(uv: vec2<f32>) -> vec4<f32> {\n\
         \x20   let z = uv.x * 0.0;\n\
         \x20   return vec4<f32>(z / z, 1.0 / z, -1.0 / z, z / z);\n\
         }\n",
    );
    let (pipeline, _) = fx
        .shader_pipeline(&ctx, 2, p.source_hash, &p.assembled)
        .expect("it compiles; it is the answer that is nonsense");
    let out = fx.custom_shader(
        &ctx,
        &pipeline,
        &tex,
        &tex,
        None,
        None,
        w,
        h,
        &header(w, h),
        &p.pack(lumit_core::fx::Params::new(&[])),
    );
    let back = readback_linear_f32(&ctx, &out, w, h).expect("readback");
    assert!(
        back.iter().all(|v| v.is_finite()),
        "one poisoned pixel becomes a black composition three effects later"
    );
}

#[test]
fn one_pipeline_per_source_hash() {
    let Ok(ctx) = GpuContext::headless() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let a = program(INVERT);
    let b = program(EVERYTHING);
    // Two instances, one source: one compile.
    fx.shader_pipeline(&ctx, 10, a.source_hash, &a.assembled)
        .unwrap();
    fx.shader_pipeline(&ctx, 11, a.source_hash, &a.assembled)
        .unwrap();
    assert_eq!(fx.shader_compiles(), 1, "two layers share one pipeline");
    // Two sources: two.
    fx.shader_pipeline(&ctx, 12, b.source_hash, &b.assembled)
        .unwrap();
    assert_eq!(fx.shader_compiles(), 2);
}

#[test]
fn two_instances_of_one_source_keep_their_own_uniforms() {
    let Ok(ctx) = GpuContext::headless() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (8u32, 8u32);
    let tex = upload_linear_f32(&ctx, &picture(w, h), w, h);
    let p = program(
        "struct Params {\n  /// @slider(0, 4) @default(1) Gain\n  gain: f32,\n}\n\
         fn shade(uv: vec2<f32>) -> vec4<f32> { return lumit_sample(uv) * p.gain; }",
    );
    let (pipeline, _) = fx
        .shader_pipeline(&ctx, 20, p.source_hash, &p.assembled)
        .unwrap();
    let (twin, _) = fx
        .shader_pipeline(&ctx, 21, p.source_hash, &p.assembled)
        .unwrap();
    assert_eq!(fx.shader_compiles(), 1, "one pipeline");
    let id = lumit_core::fx::ParamId::new("gain");
    let draw = |pl: &wgpu::ComputePipeline, gain: f32| {
        let entries = [(id, lumit_core::fx::Value::Float(gain))];
        let out = fx.custom_shader(
            &ctx,
            pl,
            &tex,
            &tex,
            None,
            None,
            w,
            h,
            &header(w, h),
            &p.pack(lumit_core::fx::Params::new(&entries)),
        );
        readback_linear_f32(&ctx, &out, w, h).expect("readback")
    };
    let one = draw(&pipeline, 1.0);
    let two = draw(&twin, 2.0);
    assert!(
        one.iter().zip(&two).any(|(a, b)| (a - b).abs() > 1e-3),
        "and two pictures: the uniform is per dispatch, not per pipeline"
    );
}

#[test]
fn a_broken_edit_keeps_the_last_good_pipeline_interactively_and_never_on_export() {
    let Ok(ctx) = GpuContext::headless() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let good = program(INVERT);
    // The export and headless answer, which is the default: no fallback at all.
    fx.shader_pipeline(&ctx, 30, good.source_hash, &good.assembled)
        .expect("the good one compiles");
    let broken = program("fn shade(uv: vec2<f32>) -> vec4<f32> { return nope(uv); }");
    let refused = fx.shader_pipeline(&ctx, 30, broken.source_hash, &broken.assembled);
    assert!(
        refused.is_err(),
        "an export renders identity and logs the error, never yesterday's shader"
    );

    // Interactively, the last pipeline that compiled keeps drawing, with the
    // compiler's message beside it — and the caller is told, which is what makes
    // such a frame showable but not cacheable.
    fx.allow_stale_shaders(true);
    let (_, badge) = fx
        .shader_pipeline(&ctx, 30, broken.source_hash, &broken.assembled)
        .expect("the last good one is still there");
    let badge = badge.expect("a stale picture always says so");
    assert!(!badge.is_empty(), "and carries the compiler's own sentence");

    // An instance that has never compiled anything has nothing to fall back to.
    assert!(fx
        .shader_pipeline(&ctx, 31, broken.source_hash, &broken.assembled)
        .is_err());
}
