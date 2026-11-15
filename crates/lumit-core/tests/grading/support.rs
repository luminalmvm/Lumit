//! What every grading test builds with: effects set by row id, a stack run on
//! the CPU, a comp of plates, and pixel checks.

use std::sync::Arc;

use lumit_core::anim::Property;
use lumit_core::expression::ExpressionContext;
use lumit_core::fx::markers::MarkerContext;
use lumit_core::fx::params::ResolvedStack;
use lumit_core::fx::resolved::resolve_stack;
use lumit_core::grade::Shot;
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, LayerKind, LinearColour, MotionBlur,
    ProjectItem, TransformGroup,
};
use lumit_core::time::{Duration, FrameRate, Rational};
use uuid::Uuid;

/// A number row.
pub fn f(v: f64) -> EffectValue {
    EffectValue::Float(Property::fixed(v))
}

/// A colour row; alpha stays 1.
pub fn rgb(c: [f64; 3]) -> EffectValue {
    EffectValue::Colour([
        Property::fixed(c[0]),
        Property::fixed(c[1]),
        Property::fixed(c[2]),
        Property::fixed(1.0),
    ])
}

/// Effect `name` with `rows` set by id. A row the effect doesn't declare is a
/// mistake in the test, so it fails loudly.
pub fn fx(name: &str, rows: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = lumit_core::fx::builtins::instantiate(name).unwrap();
    for (id, value) in rows {
        let row = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("{name} has no row {id}"));
        row.value = value.clone();
    }
    e
}

/// A Correction worked in scene linear, so its sums read straight off the
/// pixels, with `rows` on top.
pub fn linear(rows: &[(&str, EffectValue)]) -> EffectInstance {
    let mut all = vec![("space", EffectValue::Choice(2))];
    all.extend(rows.iter().cloned());
    fx("correction", &all)
}

/// `stack` resolved at time nought, full size.
pub fn resolve(stack: &[EffectInstance]) -> ResolvedStack {
    resolve_stack(
        stack,
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    )
}

/// `img` through `stack` on the CPU, as the degradation rung draws it.
pub fn run(stack: &[EffectInstance], img: &[f32], w: u32, h: u32) -> Vec<f32> {
    let mut out = img.to_vec();
    lumit_core::fx::cpu::apply_stack(&mut out, w, h, &resolve(stack));
    out
}

/// One row of opaque pixels, one colour each.
pub fn strip(colours: &[[f32; 3]]) -> Vec<f32> {
    colours
        .iter()
        .flat_map(|c| [c[0], c[1], c[2], 1.0])
        .collect()
}

/// A `w` by `h` picture of one opaque colour.
pub fn flat(w: u32, h: u32, c: [f32; 3]) -> Vec<f32> {
    strip(&vec![c; (w * h) as usize])
}

/// Pixel `i`'s colour.
pub fn at(img: &[f32], i: usize) -> [f32; 3] {
    [img[i * 4], img[i * 4 + 1], img[i * 4 + 2]]
}

/// Pixel `(x, y)`'s colour in a picture `w` wide.
pub fn at_xy(img: &[f32], w: u32, x: u32, y: u32) -> [f32; 3] {
    at(img, (y * w + x) as usize)
}

/// Each channel within `tol`.
pub fn assert_near(got: [f32; 3], want: [f32; 3], tol: f32, what: &str) {
    for c in 0..3 {
        assert!(
            (got[c] - want[c]).abs() <= tol,
            "{what}: got {got:?}, want {want:?}"
        );
    }
}

/// Rec. 709 luma.
pub fn luma(v: [f32; 3]) -> f32 {
    0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]
}

/// Smoothstep, as the modules write it.
pub fn ss(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A comp of adjustment layers, one a plate, each graded by `grade()` and
/// starting a second after the last. Answers the document and each plate as a
/// shot.
pub fn comp_of(plates: usize, grade: impl Fn() -> Vec<EffectInstance>) -> (Document, Vec<Shot>) {
    let comp_id = Uuid::now_v7();
    let mut shots = Vec::new();
    let mut layers = Vec::new();
    for i in 0..plates {
        let mut layer = lumit_core::edits::base_layer(
            format!("plate {i}"),
            LayerKind::Adjustment,
            Rational::new(4, 1).unwrap(),
            TransformGroup::default(),
        );
        layer.start_offset.0 = Rational::new(i as i64, 1).unwrap();
        layer.effects = grade();
        shots.push(Shot {
            comp: comp_id,
            layer: layer.id,
            clip: None,
        });
        layers.push(layer);
    }
    let comp = Composition {
        id: comp_id,
        name: "Comp".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(25, 1).unwrap(),
        duration: Duration(Rational::new(8, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        groups: Vec::new(),
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        beat_grid: None,
        graph: None,
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(comp));
    (doc, shots)
}

/// `shot`'s own stack.
pub fn stack_of(doc: &Document, shot: Shot) -> Vec<EffectInstance> {
    lumit_core::grade::read(doc, shot).unwrap().0.to_vec()
}
