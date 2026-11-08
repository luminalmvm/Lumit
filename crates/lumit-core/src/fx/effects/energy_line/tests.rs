use super::*;
use crate::anim::Property;
use crate::expression::ExpressionContext;
use crate::fx::markers::MarkerContext;
use std::sync::Arc;

fn set(e: &mut EffectInstance, id: &str, v: f64) {
    if let Some(p) = e.params.iter_mut().find(|p| p.id == id) {
        p.value = EffectValue::Float(Property::fixed(v));
    }
}

fn drawn(e: &EffectInstance, w: u32, h: u32, lt: f64) -> Vec<f32> {
    drawn_over(e, w, h, lt, 0.0)
}

fn drawn_over(e: &EffectInstance, w: u32, h: u32, lt: f64, ground: f32) -> Vec<f32> {
    let ops = crate::fx::resolved::resolve_stack(
        std::slice::from_ref(e),
        lt,
        (w as f32).hypot(h as f32),
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    let mut rgba = vec![ground; (w * h * 4) as usize];
    if let Some(op) = ops.get(0) {
        EnergyLineDef.apply_cpu(&mut rgba, w, h, op.params);
    }
    rgba
}

/// A line lights its own pixels white-hot with the glow colour round them,
/// and leaves the far corner dark. Mix 0 draws nothing, nothing lit leaves
/// nothing of the picture when the light is all there is, and the noise and
/// the flicker move the picture on from one moment to the next.
#[test]
fn a_line_draws_a_hot_core_inside_its_glow() {
    let (w, h) = (96u32, 48u32);
    let mut e = crate::fx::builtins::instantiate("energy_line").unwrap_or_else(|| unreachable!());
    for (id, v) in [
        ("start_x", 16.0),
        ("start_y", 24.0),
        ("end_x", 80.0),
        ("end_y", 24.0),
        ("core_size", 6.0),
        ("glow_size", 10.0),
    ] {
        set(&mut e, id, v);
    }
    let px = |rgba: &[f32], x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    let still = drawn(&e, w, h, 0.0);
    let core = px(&still, 48, 24);
    assert!(
        core[0] > 0.9 && core[3] > 0.99,
        "the core is white: {core:?}"
    );
    let glow = px(&still, 48, 31);
    assert!(
        glow[2] > 0.05 && glow[2] > glow[0] * 3.0,
        "the glow is blue: {glow:?}"
    );
    // Straight alpha, so the light is the colour times its alpha.
    let far = px(&still, 1, 1);
    assert!(
        far[2] * far[3] < glow[2] * glow[3] * 0.25,
        "the far corner stays dark: {far:?}"
    );

    set(&mut e, "mix", 0.0);
    assert!(
        drawn(&e, w, h, 0.0).iter().all(|v| *v == 0.0),
        "Mix 0 draws nothing"
    );
    set(&mut e, "mix", 100.0);

    set(&mut e, "end", 0.0);
    assert!(
        drawn_over(&e, w, h, 0.0, 0.5).iter().all(|v| *v == 0.0),
        "nothing lit leaves nothing"
    );
    set(&mut e, "end", 100.0);

    set(&mut e, "core_distortion", 3.0);
    set(&mut e, "glow_distortion", 4.0);
    set(&mut e, "flicker", 50.0);
    assert!(moves_with_time(&e));
    assert_ne!(drawn(&e, w, h, 0.0), drawn(&e, w, h, 0.5));
}

/// Two paths in one run light one after another, or all at once, each from
/// its own start.
#[test]
fn a_run_of_paths_draws_on_in_turn_or_together() {
    use crate::mask::MaskPolyline;
    // Two 100 px lines, the step between them having no length.
    let run = MaskPolyline {
        points: vec![[0.0, 0.0], [100.0, 0.0], [0.0, 50.0], [100.0, 50.0]],
        arc: vec![0.0, 100.0, 100.0, 200.0],
        ..MaskPolyline::default()
    };
    let in_turn = geometry::pieces(&run, 1.0, [0.0, 40.0, 0.0], 0, 4.0);
    let together = geometry::pieces_each(&run, 1.0, [0.0, 40.0, 0.0], 0, 4.0);
    let reach = |p: &geometry::Pieces| p.segs.iter().map(|s| s[2]).fold(0.0f32, f32::max);
    assert_eq!(in_turn.segs.len(), 1, "only the first path is lit");
    assert!((reach(&in_turn) - 80.0).abs() < 1e-4);
    assert_eq!(together.segs.len(), 2, "both paths are lit");
    assert!(together.segs.iter().all(|s| (s[2] - 40.0).abs() < 1e-4));
}

/// Picking a look writes its rows, and editing one of them puts the Preset
/// row back to Custom so it never names a look the light has left.
#[test]
fn a_preset_writes_its_look_and_an_edit_makes_it_custom() {
    let plain = crate::fx::builtins::instantiate("energy_line").unwrap_or_else(|| unreachable!());
    let mut neon = plain.clone();
    for p in &mut neon.params {
        if p.id == "preset" {
            p.value = EffectValue::Choice(3);
        }
    }
    let wrote = EnergyLineDef
        .settle(Some(&plain), &neon)
        .unwrap_or_else(|| unreachable!());
    assert!(wrote
        .rows
        .iter()
        .any(|(id, v)| *id == "core_size" && *v == Value::Float(6.0)));

    let mut edited = neon.clone();
    set(&mut edited, "glow_size", 99.0);
    let back = EnergyLineDef
        .settle(Some(&neon), &edited)
        .unwrap_or_else(|| unreachable!());
    assert_eq!(back.rows, vec![("preset", Value::Choice(0))]);
    assert!(EnergyLineDef.settle(Some(&plain), &plain).is_none());
}
