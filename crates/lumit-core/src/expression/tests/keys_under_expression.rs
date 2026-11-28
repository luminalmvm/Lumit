use super::*;
use crate::anim::{Animation, Keyframe, Property, SideInterp};
use crate::time::Rational;

fn key(seconds: i64, value: f64) -> Keyframe {
    Keyframe {
        time: Rational::new(seconds, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

/// 0 at 0 s rising to 10 at 2 s, then a JavaScript expression on it.
fn keyed_with(expression: &str) -> Property {
    let mut p = Property::fixed(0.0);
    p.animation = Animation::Keyframed(vec![key(0, 0.0), key(2, 10.0)]);
    p.set_animation(Animation::Expression(expression.into()));
    p.set_expression_language(Language::JavaScript);
    p
}

fn at(p: &Property, t: f64) -> f64 {
    let mut c = ExpressionContext::detached();
    c.comp_time = t;
    p.value_at_with_context(t, Arc::new(c))
}

#[test]
fn loop_out_repeats_the_keys() {
    let p = keyed_with("loopOut(\"cycle\")");
    // 3 s is 1 s into the second lap: 5, as at 1 s.
    assert!((at(&p, 3.0) - 5.0).abs() < 1e-9, "{}", at(&p, 3.0));
    assert!((at(&p, 5.5) - 7.5).abs() < 1e-9, "{}", at(&p, 5.5));
    let p = keyed_with("valueAtTime(1)");
    assert!((at(&p, 7.0) - 5.0).abs() < 1e-9);
}

#[test]
fn a_number_over_the_expression_leaves_only_the_number() {
    let mut p = keyed_with("value");
    p.set_animation(Animation::Static(3.0));
    assert_eq!(p.animation, Animation::Static(3.0));
    assert!(p.extra.get(SLOT_KEY).is_none());
    // Remove expression, or baking into keys, writes keys and replaces both.
    let mut p = keyed_with("value");
    p.set_animation(Animation::Keyframed(vec![key(0, 1.0)]));
    assert!(matches!(&p.animation, Animation::Keyframed(k) if k.len() == 1));
    assert!(p.extra.get(SLOT_KEY).is_none());
}

#[test]
fn editing_the_text_keeps_the_keys_and_they_survive_a_save() {
    let mut p = keyed_with("value");
    p.set_animation(Animation::Expression("value + 1".into()));
    assert_eq!(p.keys_under_expression().map(|k| k.len()), Some(2));
    let back: Property = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back.keys_under_expression(), p.keys_under_expression());
    assert!((at(&back, 2.0) - 11.0).abs() < 1e-9);
}

#[test]
fn a_retime_loops_with_loop_out() {
    let mut layer = super::driven_layer("Looped", "0");
    let mut retime = Property::fixed(0.0);
    retime.animation = Animation::Keyframed(vec![key(0, 0.0), key(2, 2.0)]);
    retime.set_animation(Animation::Expression("loopOut(\"cycle\")".into()));
    retime.set_expression_language(Language::JavaScript);
    layer.retime = Some(retime);
    assert!((layer.source_time_at(1.0) - 1.0).abs() < 1e-9);
    assert!((layer.source_time_at(3.0) - 1.0).abs() < 1e-9);
    assert!((layer.source_time_at(5.5) - 1.5).abs() < 1e-9);
    // Flow reads the speed, and a still one keeps it off.
    assert!((layer.speed_at(5.5) - 1.0).abs() < 1e-6);
    // The layer starting later moves the keys with it.
    layer.start_offset.0 = Rational::new(10, 1).unwrap();
    assert!((layer.source_time_at(3.0) - 1.0).abs() < 1e-9);
}
