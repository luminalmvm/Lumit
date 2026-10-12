//! The Custom shader's GPU-side tests (docs/impl/custom-shader.md §8 items 11,
//! 12, 15, 16, 17, 18, 19 and the line-number half of 10).
//!
//! **In plain terms.** The half of the effect that needs a graphics card, and the
//! half that only needs the shader compiler. The compiler half runs everywhere,
//! including on a machine and a CI runner with no card, which is the whole point
//! of validating a user's text through naga rather than at pipeline creation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::fx::shader;
use lumit_gpu::fx::{readback_linear_f32, upload_linear_f32, validate};

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
    // The host's own wrapper, round every fixture, through the validator — so a
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

// ------------------------------------------------- §8 items 12, 15, 16, 17, 18

#[test]
fn a_golden_shader_renders_deterministically() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        "the same inputs render bit-identically twice"
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

// ------------------------------------------------------- §8 items 22 and 23

/// A small builder so a test reads as the graph it draws.
fn gnode(id: u32, kind: &str) -> lumit_core::fx::shader::graph::ShaderNode {
    lumit_core::fx::shader::graph::ShaderNode {
        id,
        kind: kind.to_owned(),
        settings: serde_json::Map::new(),
    }
}

fn gedge(
    from: u32,
    from_port: u32,
    to: u32,
    to_port: u32,
) -> lumit_core::fx::shader::graph::ShaderEdge {
    lumit_core::fx::shader::graph::ShaderEdge {
        from,
        from_port,
        to,
        to_port,
    }
}

/// Every box in the v1 vocabulary, compiled and taken through the validator —
/// so the graph compiler cannot emit WGSL the validator refuses. No card.
#[test]
fn a_graph_of_every_node_assembles_and_validates() {
    use lumit_core::fx::shader::graph::{ShaderGraph, ShaderNode};
    let mut nodes = vec![gnode(1, "picture"), gnode(2, "luminance")];
    let mut edges = vec![gedge(1, 0, 2, 0)];
    let mut id = 100u32;
    let mut spine = 1u32;
    for kind in ["premultiply", "unpremultiply", "tint", "blend"] {
        id += 1;
        let mut n = gnode(id, kind);
        if kind == "blend" {
            n.settings = serde_json::json!({"mode": "screen"})
                .as_object()
                .expect("an object")
                .clone();
        }
        nodes.push(n);
        edges.push(gedge(spine, 0, id, 0));
        spine = id;
    }
    let mut scalar = 2u32;
    for kind in [
        "add",
        "subtract",
        "multiply",
        "divide",
        "modulo",
        "mix",
        "clamp",
        "saturate",
        "pow",
        "sqrt",
        "abs",
        "sign",
        "min",
        "max",
        "floor",
        "ceil",
        "fract",
        "step",
        "smoothstep",
        "sin",
        "cos",
        "atan2",
        "length",
        "distance",
    ] {
        id += 1;
        nodes.push(gnode(id, kind));
        edges.push(gedge(scalar, 0, id, 0));
        scalar = id;
    }
    nodes.push(gnode(3, "uv"));
    nodes.push(gnode(4, "split"));
    edges.push(gedge(3, 0, 4, 0));
    nodes.push(gnode(5, "combine2"));
    edges.push(gedge(4, 0, 5, 0));
    edges.push(gedge(4, 1, 5, 1));
    nodes.push(gnode(6, "normalize"));
    edges.push(gedge(5, 0, 6, 0));
    nodes.push(gnode(7, "dot"));
    edges.push(gedge(6, 0, 7, 0));
    edges.push(gedge(5, 0, 7, 1));
    let mut sw = gnode(8, "swizzle");
    sw.settings = serde_json::json!({"pattern": "yx"})
        .as_object()
        .expect("an object")
        .clone();
    nodes.push(sw);
    edges.push(gedge(5, 0, 8, 0));
    nodes.push(gnode(9, "picture2"));
    nodes.push(gnode(10, "sample"));
    edges.push(gedge(9, 1, 10, 0));
    edges.push(gedge(8, 0, 10, 1));
    nodes.push(gnode(11, "matte"));
    nodes.push(gnode(12, "time"));
    nodes.push(gnode(13, "seed"));
    nodes.push(gnode(14, "combine3"));
    edges.push(gedge(11, 0, 14, 0));
    edges.push(gedge(12, 0, 14, 1));
    edges.push(gedge(13, 0, 14, 2));
    let mut param: ShaderNode = gnode(15, "param");
    param.settings = serde_json::json!({
        "id": "amount", "kind": "slider", "min": 0, "max": 1, "default": 1
    })
    .as_object()
    .expect("an object")
    .clone();
    nodes.push(param);
    edges.push(gedge(15, 0, spine, 2));
    edges.push(gedge(scalar, 0, spine, 1));
    nodes.push(gnode(16, "result"));
    edges.push(gedge(spine, 0, 16, 0));

    let text = lumit_core::fx::shader::compile::compile(&ShaderGraph {
        nodes,
        edges,
        layout: Vec::new(),
    })
    .expect("the whole vocabulary compiles");
    let p = program(&text);
    if let Err(why) = validate(&p.assembled) {
        panic!("the vocabulary did not validate:\n{why}\n\n{}", p.assembled);
    }
}

#[test]
fn a_nan_returned_by_a_shader_never_leaves_the_effect() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
fn a_broken_edit_keeps_the_last_good_pipeline_interactively_and_never_on_export() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
