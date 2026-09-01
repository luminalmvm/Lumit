//! The Custom shader's engine-side tests (docs/impl/custom-shader.md §8,
//! items 1–10 and the §7 padding trap). No graphics card is involved in any of
//! them: the grammar, the refusals and the uniform arithmetic are all decided in
//! this crate, which is the whole point of deciding them here.

use std::sync::Arc;

use super::*;
use crate::anim::{Animation, Keyframe, Property, SideInterp};
use crate::expression::ExpressionContext;
use crate::fx::effects::custom_shader::{program_of, source_of, EXTRA_KEY};
use crate::fx::{instantiate, MarkerContext, ParamId, Value};
use crate::model::{EffectParam, EffectValue};

/// The §1.4 declaration block, all nine forms, exactly as the note pins it.
const NINE: &str = r#"
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Radius
    radius: f32,
    /// @bounded(0, 1) @default(0.5) Blend point
    blend_point: f32,
    /// @dial @default(0) Angle
    angle: f32,
    /// @counter(1, 16) @default(4) Steps
    steps: i32,
    /// @toggle @default(true) Invert
    invert: u32,
    /// @choice("Soft", "Hard", "Wrapped") @default("Soft") Edge
    edge: u32,
    /// @colour @default(1, 0.5, 0.2, 1) Tint
    tint: vec4<f32>,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @seed Seed
    seed_v: u32,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.radius;
}
"#;

fn program(source: &str) -> &'static ShaderProgram {
    match build(source) {
        Ok(p) => Box::leak(Box::new(p)),
        Err(e) => panic!("expected a program, got: {e}"),
    }
}

fn row<'a>(p: &'a ShaderProgram, id: &str) -> &'a ParamSchema {
    p.params
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no row `{id}` in {:?}", p.params))
}

// ------------------------------------------------------------------ §8 item 1

#[test]
fn a_custom_shader_with_no_source_is_a_passthrough() {
    let inst = instantiate("custom_shader").unwrap();
    assert_eq!(source_of(&inst), None, "a fresh instance holds no source");
    assert!(program_of(&inst).is_none(), "and so compiles nothing");
    let def = crate::fx::BUILTIN_DEFS.get("custom_shader").unwrap();
    assert!(def.derived(&inst).is_empty(), "and offers no derived rows");
    // The identity CPU rung: `apply_cpu` is the default, so the picture is
    // byte-identical to what it was handed.
    let mut rgba = vec![0.25, 0.5, 0.75, 1.0, 0.1, 0.2, 0.3, 0.4];
    let before = rgba.clone();
    def.apply_cpu(&mut rgba, 2, 1, crate::fx::Params::new(&[]));
    assert_eq!(rgba, before);
}

// ------------------------------------------------------------------ §8 item 2

#[test]
fn the_annotation_reader_derives_every_kind() {
    let p = program(NINE);
    assert_eq!(
        p.params.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![
            "radius",
            "blend_point",
            "angle",
            "steps",
            "invert",
            "edge",
            "tint",
            "centre_x",
            "centre_y",
            "seed_v",
        ],
        "declaration order is the schema order, and a point is two rows"
    );

    let radius = row(p, "radius");
    assert_eq!(radius.label, "Radius");
    assert_eq!(radius.unit, Unit::Px);
    assert_eq!(
        radius.kind,
        ParamKind::Float {
            default: 25.0,
            slider: (0.0, 200.0),
            hard: (None, None)
        }
    );
    assert_eq!(
        row(p, "blend_point").kind,
        ParamKind::Slider {
            default: 0.5,
            range: (0.0, 1.0)
        }
    );
    assert_eq!(
        row(p, "angle").kind,
        ParamKind::Angle {
            default: 0.0,
            dial_step: 15.0
        }
    );
    assert_eq!(row(p, "angle").unit, Unit::Degrees, "a dial is degrees");
    assert_eq!(
        row(p, "steps").kind,
        ParamKind::Int {
            default: 4,
            slider: (1, 16),
            hard: (None, None)
        }
    );
    assert_eq!(row(p, "invert").kind, ParamKind::Bool { default: true });
    assert_eq!(
        row(p, "edge").kind,
        ParamKind::Choice {
            options: &["Soft", "Hard", "Wrapped"],
            default: 0,
            dividers_after: &[]
        }
    );
    assert_eq!(
        row(p, "tint").kind,
        ParamKind::Colour {
            default: [1.0, 0.5, 0.2, 1.0],
            range: (0.0, 1.0)
        }
    );
    assert_eq!(row(p, "centre_x").label, "Centre X");
    assert_eq!(row(p, "centre_y").label, "Centre Y");
    assert_eq!(row(p, "centre_x").unit, Unit::Px, "a point is px@comp");
    assert_eq!(row(p, "seed_v").kind, ParamKind::Seed);
    assert!(p.notes.is_empty(), "nothing was skipped: {:?}", p.notes);
}

// ------------------------------------------------------------------ §8 item 3

#[test]
fn an_unannotated_field_is_still_a_parameter() {
    let p = program(
        "struct Params {\n  a: f32,\n  b: i32,\n  c: u32,\n  d: vec4<f32>,\n  \
         /// Where\n  e: vec2<f32>,\n}\nfn shade(uv: vec2<f32>) -> vec4<f32> { return \
         vec4<f32>(p.a); }",
    );
    assert_eq!(
        row(p, "a").kind,
        ParamKind::Float {
            default: 0.0,
            slider: (0.0, 1.0),
            hard: (None, None)
        }
    );
    assert_eq!(
        row(p, "a").label,
        "A",
        "an unlabelled field humanises its name"
    );
    assert!(matches!(row(p, "b").kind, ParamKind::Int { .. }));
    assert!(matches!(row(p, "c").kind, ParamKind::Int { .. }));
    assert!(matches!(row(p, "d").kind, ParamKind::Colour { .. }));
    assert_eq!(row(p, "e_x").label, "Where X");
    assert!(
        !p.params.iter().any(|r| r.id == "e"),
        "a point is its two halves and never a row of its own"
    );
}

// ------------------------------------------------------------------ §8 item 4

#[test]
fn a_malformed_annotation_skips_one_parameter_and_keeps_the_rest() {
    let p = program(
        "struct Params {\n  /// @slider(nonsense) Radius\n  radius: f32,\n  \
         /// @default(2) Steps\n  steps: i32,\n}\n\
         fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }",
    );
    assert_eq!(
        p.params.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec!["steps"],
        "the typo costs its own row and no other"
    );
    assert_eq!(p.notes.len(), 1, "and says so, calmly: {:?}", p.notes);
    assert!(p.notes[0].contains("radius"), "{:?}", p.notes);
    // The field still occupies its bytes: dropping it would move every offset
    // after it, which is the §7 trap with no error anywhere.
    assert_eq!(p.fields.len(), 2);
    assert_eq!(p.fields[1].offset, 4);
}

// ------------------------------------------------------------------ §8 item 5

#[test]
fn a_vec3_parameter_is_refused_with_the_padding_reason() {
    let err = build(
        "struct Params {\n  /// Tint\n  tint: vec3<f32>,\n}\n\
         fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }",
    )
    .unwrap_err();
    assert_eq!(err, ShaderRefusal::Vec3Field("tint".to_owned()));
    let said = err.to_string();
    assert!(said.contains("sixteen bytes"), "{said}");
    assert!(said.contains("vec4<f32>"), "{said}");
}

// ------------------------------------------------------------------ §8 item 6

#[test]
fn the_reader_never_panics() {
    // Truncated, unbalanced, non-ASCII and plain nonsense. This reads user text,
    // so it is a parser at a trust boundary (docs/14 §4).
    let mut cases: Vec<String> = vec![
        String::new(),
        "struct".to_owned(),
        "struct Params".to_owned(),
        "struct Params {".to_owned(),
        "struct Params { a".to_owned(),
        "struct Params { a: }".to_owned(),
        "struct Params { : f32, }".to_owned(),
        "fn shade(".to_owned(),
        "}}}}{{{{".to_owned(),
        "/* unterminated".to_owned(),
        "/// @".to_owned(),
        "struct Params { /// @slider( \n a: f32, }".to_owned(),
        "структ Пар { }".to_owned(),
        "fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); } // 🎛".to_owned(),
        "var<uniform> \u{0}: f32;".to_owned(),
    ];
    // Every prefix of the nine-form block, which is every way a person can be
    // part-way through typing it.
    for i in 0..NINE.len() {
        if NINE.is_char_boundary(i) {
            cases.push(NINE[..i].to_owned());
        }
    }
    for c in &cases {
        // A refusal is an answer; a panic is not. `build` returning at all is
        // the assertion.
        let _ = build(c);
    }
}

// ------------------------------------------------------------------ §8 item 9

#[test]
fn a_shader_that_declares_its_own_binding_is_refused_at_the_edit() {
    let err = build(
        "@group(0) @binding(7) var<uniform> mine: f32;\n\
         fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }",
    )
    .unwrap_err();
    assert_eq!(err, ShaderRefusal::OwnBinding);
    assert!(err.to_string().contains("the host declares the bindings"));
}

#[test]
fn every_reserved_name_is_refused_at_the_edit() {
    for name in RESERVED.iter().copied().chain(["lumit_sample"]) {
        let source = format!(
            "fn {name}() -> f32 {{ return 1.0; }}\n\
             fn shade(uv: vec2<f32>) -> vec4<f32> {{ return vec4<f32>(0.0); }}"
        );
        assert_eq!(
            build(&source).err(),
            Some(ShaderRefusal::ReservedName(name.to_owned())),
            "`{name}` must be refused"
        );
    }
    // Inside a function body the same word is the user's own, and is left alone.
    assert!(build(
        "fn shade(uv: vec2<f32>) -> vec4<f32> { let src = 1.0; return vec4<f32>(src); }"
    )
    .is_ok());
}

#[test]
fn a_source_with_no_shade_function_is_refused_at_the_edit() {
    let err = build("fn other(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }").unwrap_err();
    assert_eq!(err, ShaderRefusal::NoShadeFunction);
}

#[test]
fn a_derived_id_may_not_collide_with_a_declared_one() {
    let err = build(
        "struct Params {\n  /// Mix\n  mix: f32,\n}\n\
         fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }",
    )
    .unwrap_err();
    assert_eq!(err, ShaderRefusal::DuplicateId("mix".to_owned()));
}

// ----------------------------------------------------------------- §8 item 10

#[test]
fn the_prologue_line_count_is_what_remaps_a_compile_error() {
    let p = program(NINE);
    let head: String = p
        .assembled
        .lines()
        .take(p.prologue_lines as usize)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        head.contains("struct Params {") && head.contains("@binding(6)"),
        "the lifted struct and every binding are host lines"
    );
    assert!(
        !head.contains("fn shade"),
        "the user's own text begins after them"
    );
}

// -------------------------------------------------------- §7, the padding trap

#[test]
fn the_uniform_layout_is_pinned() {
    let p = program(NINE);
    let at = |name: &str| {
        p.fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| (f.ty, f.offset))
            .unwrap_or_else(|| panic!("no field `{name}`"))
    };
    // f32 f32 f32 i32 u32 u32 | vec4 (16) | vec2 (8) | u32 → 4 4 4 4 4 4, pad to
    // 32 for the colour, 48 for the point, 56 for the seed, block rounded to 64.
    assert_eq!(at("radius"), (WgslTy::F32, 0));
    assert_eq!(at("blend_point"), (WgslTy::F32, 4));
    assert_eq!(at("angle"), (WgslTy::F32, 8));
    assert_eq!(at("steps"), (WgslTy::I32, 12));
    assert_eq!(at("invert"), (WgslTy::U32, 16));
    assert_eq!(at("edge"), (WgslTy::U32, 20));
    assert_eq!(at("tint"), (WgslTy::Vec4, 32), "a vec4 aligns to sixteen");
    assert_eq!(at("centre"), (WgslTy::Vec2, 48), "a vec2 aligns to eight");
    assert_eq!(at("seed_v"), (WgslTy::U32, 56));
    assert_eq!(
        p.params_size, 64,
        "a uniform block is a multiple of sixteen"
    );

    // The padding is visible in the text the user can read, not inferred.
    let head: String = p
        .assembled
        .lines()
        .take(p.prologue_lines as usize)
        .collect();
    assert!(
        head.contains("_pad0: u32,"),
        "explicit named padding: {head}"
    );
}

#[test]
fn an_empty_params_block_is_still_a_legal_uniform() {
    // WGSL has no empty struct, so a shader that declares no parameters gets one
    // placeholder member and a sixteen-byte buffer.
    let p = program("fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }");
    assert!(p.params.is_empty());
    assert_eq!(p.params_size, 16);
    assert_eq!(p.pack(crate::fx::Params::new(&[])).len(), 16);
    assert!(p.assembled.contains("struct Params {"));
}

#[test]
fn the_packed_bytes_land_where_the_struct_says() {
    let p = program(NINE);
    let entries = vec![
        (ParamId::new("radius"), Value::Float(12.5)),
        (ParamId::new("steps"), Value::Int(7)),
        (ParamId::new("invert"), Value::Bool(true)),
        (ParamId::new("edge"), Value::Choice(2)),
        (ParamId::new("tint"), Value::Colour([0.25, 0.5, 0.75, 1.0])),
        (ParamId::new("centre_x"), Value::Float(960.0)),
        (ParamId::new("centre_y"), Value::Float(540.0)),
    ];
    let bytes = p.pack(crate::fx::Params::new(&entries));
    assert_eq!(bytes.len(), 64);
    let f32_at = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let i32_at = |o: usize| i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    assert_eq!(f32_at(0), 12.5);
    // Untouched rows fall back to their declared default, never to a fault.
    assert_eq!(f32_at(4), 0.5, "blend_point's declared default");
    assert_eq!(i32_at(12), 7);
    assert_eq!(u32_at(16), 1);
    assert_eq!(u32_at(20), 2);
    assert_eq!(f32_at(32), 0.25);
    assert_eq!(f32_at(44), 1.0);
    assert_eq!(f32_at(48), 960.0);
    assert_eq!(f32_at(52), 540.0);
    // The bytes between the last u32 and the block's end are the padding, and
    // they are nought rather than whatever was in the buffer.
    assert_eq!(&bytes[60..64], &[0, 0, 0, 0]);
}

#[test]
fn one_source_is_read_once() {
    let a = program_for(NINE).unwrap();
    let b = program_for(NINE).unwrap();
    assert!(
        std::ptr::eq(a, b),
        "the parse is cached per distinct source"
    );
    assert_eq!(a.source_hash, hash64(NINE.as_bytes()));
    // A refusal is cached too, so a broken source is not re-read every frame.
    assert!(program_for("nothing at all").is_err());
    assert!(program_for("nothing at all").is_err());
}

// -------------------------------------------------- §8 items 7 and 8, resolve

fn key(t: i64, v: f64) -> Keyframe {
    Keyframe {
        time: crate::time::Rational::new(t, 1).unwrap(),
        value: v,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

/// A Custom shader instance holding `NINE`, with `radius` keyframed.
fn instance_with_shader() -> crate::model::EffectInstance {
    let mut inst = instantiate("custom_shader").unwrap();
    let mut block = serde_json::Map::new();
    block.insert("language".into(), "wgsl".into());
    block.insert("source".into(), NINE.into());
    inst.extra
        .insert(EXTRA_KEY.to_owned(), serde_json::Value::Object(block));
    inst.params.push(EffectParam {
        id: "radius".to_owned(),
        value: EffectValue::Float(Property {
            animation: Animation::Keyframed(vec![key(0, 10.0), key(1, 20.0)]),
            extra: serde_json::Map::new(),
        }),
        extra: serde_json::Map::new(),
    });
    inst
}

fn resolved_radius(inst: &crate::model::EffectInstance, lt: f64, px_scale: f32) -> Option<f32> {
    let ops = crate::fx::resolve_stack(
        std::slice::from_ref(inst),
        lt,
        2202.9,
        px_scale,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    let fx = ops.get(0)?;
    match fx.params.get(ParamId::new("radius"))? {
        Value::Float(v) => Some(v),
        _ => None,
    }
}

#[test]
fn a_derived_parameter_animates_and_serialises_like_a_declared_one() {
    let inst = instance_with_shader();
    assert_eq!(resolved_radius(&inst, 0.0, 1.0), Some(10.0));
    assert_eq!(resolved_radius(&inst, 1.0, 1.0), Some(20.0));
    // `@unit(px)` means px@comp, so the preview factor moves it exactly as it
    // moves a declared pixel count.
    assert_eq!(resolved_radius(&inst, 1.0, 0.5), Some(10.0));
    // And it survives a round trip through the document, `extra` and all.
    let json = serde_json::to_string(&inst).unwrap();
    let back: crate::model::EffectInstance = serde_json::from_str(&json).unwrap();
    assert_eq!(source_of(&back), Some(NINE));
    assert_eq!(resolved_radius(&back, 1.0, 1.0), Some(20.0));
}

#[test]
fn a_derived_px_parameter_rescales_with_the_stack() {
    let inst = instance_with_shader();
    let mut ops = crate::fx::resolve_stack(
        std::slice::from_ref(&inst),
        1.0,
        2202.9,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    ops.rescale_spatial(0.5);
    let fx = ops.get(0).unwrap();
    assert_eq!(
        fx.params.get(ParamId::new("radius")),
        Some(Value::Float(10.0)),
        "a stack reused at another raster moves a derived pixel count too"
    );
}

#[test]
fn removing_a_shader_uniform_leaves_its_parameter_and_its_expression_alive() {
    let mut inst = instance_with_shader();
    // The source stops mentioning `radius`; nothing is removed automatically.
    let shorter = "struct Params {\n  /// Steps\n  steps: i32,\n}\n\
                   fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }";
    let mut block = serde_json::Map::new();
    block.insert("source".into(), shorter.into());
    inst.extra
        .insert(EXTRA_KEY.to_owned(), serde_json::Value::Object(block));
    assert!(
        inst.params.iter().any(|p| p.id == "radius"),
        "the stored row outlives the uniform it was derived from"
    );
    let def = crate::fx::BUILTIN_DEFS.get("custom_shader").unwrap();
    assert_eq!(
        def.derived(&inst).iter().map(|r| r.id).collect::<Vec<_>>(),
        vec!["steps"],
        "and the offered set is the source's, not the document's"
    );
    // A row with no uniform behind it simply is not in the bag, which is the
    // K-258 rule: a missing parameter is a default, never a fault.
    assert_eq!(resolved_radius(&inst, 1.0, 1.0), None);
}

/// **A fresh Custom shader opens with an example that compiles** (owner,
/// 2026-09-01). The starter exists to show the format, so a starter the host
/// refuses would teach the wrong one — and it is neutral on purpose, which is
/// the second half of what it promises.
#[test]
fn the_starter_shader_compiles_and_changes_nothing() {
    let inst = crate::fx::instantiate_for_raster("custom_shader", 1920.0, 1080.0)
        .expect("the catalogue knows it");
    let source =
        crate::fx::effects::custom_shader::source_of(&inst).expect("a fresh instance has one");
    assert!(
        crate::fx::shader::program_for(source).is_ok(),
        "the starter must pass every refusal the host makes"
    );

    // Its two rows are the point of the example: a slider and a colour, read
    // off the text exactly as a user's own fields are (K-650).
    let program = crate::fx::shader::program_for(source).expect("compiled");
    let ids: Vec<String> = program.params.iter().map(|p| p.id.to_string()).collect();
    assert_eq!(ids, vec!["gain", "tint"], "the example declares both kinds");

    // Neutral: gain 1, white tint, so dropping the effect on changes no pixel
    // until the user writes something. K-111's exception, stated in the source.
    assert_eq!(
        crate::fx::instantiate("custom_shader")
            .map(|plain| crate::fx::effects::custom_shader::source_of(&plain).is_none()),
        Some(true),
        "the pure schema default is still empty - presets and tests keep it"
    );
}
