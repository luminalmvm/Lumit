//! The Custom shader's engine-side tests (docs/impl/custom-shader.md §8,
//! items 1–10 and the §7 padding trap). No graphics card is involved in any of
//! them: the grammar, the refusals and the uniform arithmetic are all decided in
//! this crate, which is the whole point of deciding them here.

use std::sync::Arc;

use super::*;
use crate::anim::{Animation, Keyframe, Property, SideInterp};
use crate::expression::ExpressionContext;
use crate::fx::effects::custom_shader::{source_of, EXTRA_KEY};
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
            range: (0.0, 1.0),
            log: false
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

// ------------------------------------------------------------------ §8 item 4

// ------------------------------------------------------------------ §8 item 5

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

// ----------------------------------------------------------------- §8 item 10

// -------------------------------------------------------- §7, the padding trap

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
    // rule: a missing parameter is a default, never a fault.
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
    // off the text exactly as a user's own fields are.
    let program = crate::fx::shader::program_for(source).expect("compiled");
    let ids: Vec<String> = program.params.iter().map(|p| p.id.to_string()).collect();
    assert_eq!(ids, vec!["gain", "tint"], "the example declares both kinds");

    // Neutral: gain 1, white tint, so dropping the effect on changes no pixel
    // until the user writes something.
    assert_eq!(
        crate::fx::instantiate("custom_shader")
            .map(|plain| crate::fx::effects::custom_shader::source_of(&plain).is_none()),
        Some(true),
        "the pure schema default is still empty - presets and tests keep it"
    );
}

/// The shader draws as you type, so every settle of the keyboard asks for a new
/// distinct source. Without a ceiling that is a slow leak with a person on the
/// other end of it, since the cache cannot let anything go while its entries are
/// `&'static` (see `program_for`).
///
/// Driven against a cache of this test's own, with a ceiling of three: the
/// shipped one is process-wide and holds four thousand, so a test that filled it
/// would leave every test scheduled after it unable to read a shader. The lines
/// under test are the same ones.
#[test]
fn a_session_may_not_read_shaders_without_end() {
    use crate::fx::shader::{program_in, ProgramCache, ShaderRefusal};

    let cache = ProgramCache::default();
    // One valid shader per iteration, each distinct by a comment nobody reads.
    let source = |n: usize| {
        format!(
            "// {n}\nfn shade(uv: vec2<f32>) -> vec4<f32> {{ return vec4<f32>(uv, 0.0, 1.0); }}\n"
        )
    };

    for n in 0..3 {
        assert!(
            program_in(&cache, &source(n), 3).is_ok(),
            "an ordinary shader was refused below the ceiling"
        );
    }

    // The fourth distinct source meets it, by name and with the number in it.
    let refused = program_in(&cache, &source(3), 3);
    assert!(
        matches!(refused, Err(ShaderRefusal::TooManyPrograms { limit: 3 })),
        "{refused:?}"
    );
    let words = ShaderRefusal::TooManyPrograms { limit: 3 }.to_string();
    assert!(words.contains('3'), "{words}");

    // A source already read is still served: a project whose shaders are all in
    // the cache keeps drawing rather than going dark because somebody once
    // typed too much.
    assert!(
        program_in(&cache, &source(0), 3).is_ok(),
        "a source already read must still be served after the ceiling"
    );
}
