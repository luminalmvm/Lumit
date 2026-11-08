//! Per-effect conversion tests for the warps, wipes and stylisers.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeMap;

use lumit_core::time::Rational;
use uuid::Uuid;

use super::super::time::TimeBase;
use super::*;
use crate::capture::{Ease, Keyframe as AeKey, Property as AeProp};
use crate::report::ImportReport;

const W: f64 = 1920.0;
const H: f64 = 1080.0;

/// The comp diagonal, the base AE's per-cent-of-layer radii convert
/// through — computed from the composition, never copied in as a constant.
fn diag() -> f64 {
    (W * W + H * H).sqrt()
}

/// The two layers the test composition has, by After Effects index.
fn layer_id(index: u32) -> Uuid {
    Uuid::from_u128(u128::from(index))
}

// ── building a captured instance ──

fn fx(ae: &str, name: &str, params: Vec<AeProp>) -> AeProp {
    AeProp {
        match_name: Some(ae.to_string()),
        name: Some(name.to_string()),
        enabled: Some(true),
        group: Some(params),
        ..AeProp::default()
    }
}

fn leaf(ae: &str, n: u32, name: &str, kind: &str, value: serde_json::Value) -> AeProp {
    AeProp {
        match_name: Some(format!("{ae}-{n:04}")),
        name: Some(name.to_string()),
        value_type: Some(kind.to_string()),
        value: Some(value),
        ..AeProp::default()
    }
}

fn num(ae: &str, n: u32, name: &str, v: f64) -> AeProp {
    leaf(ae, n, name, "float", serde_json::json!(v))
}

fn point(ae: &str, n: u32, name: &str, x: f64, y: f64) -> AeProp {
    leaf(ae, n, name, "point", serde_json::json!([x, y]))
}

fn colour(ae: &str, n: u32, name: &str, c: [f64; 4]) -> AeProp {
    leaf(ae, n, name, "colour", serde_json::json!(c))
}

fn layer_ref(ae: &str, n: u32, name: &str, index: i64) -> AeProp {
    leaf(ae, n, name, "layer", serde_json::json!(index))
}

/// A keyframed float: bezier keys, so a conversion has both a value and a
/// handle speed to get right.
fn keyed(ae: &str, n: u32, name: &str, keys: &[(f64, f64, f64)]) -> AeProp {
    AeProp {
        match_name: Some(format!("{ae}-{n:04}")),
        name: Some(name.to_string()),
        value_type: Some("float".to_string()),
        keyframes: Some(
            keys.iter()
                .map(|(t, v, speed)| AeKey {
                    t: Some(*t),
                    v: Some(serde_json::json!(v)),
                    in_interp: Some("BEZIER".to_string()),
                    out_interp: Some("BEZIER".to_string()),
                    in_ease: Some(vec![Ease {
                        speed: Some(*speed),
                        influence: Some(50.0),
                    }]),
                    out_ease: Some(vec![Ease {
                        speed: Some(*speed),
                        influence: Some(50.0),
                    }]),
                    ..AeKey::default()
                })
                .collect(),
        ),
        ..AeProp::default()
    }
}

// ── running one ──

struct Run {
    inst: EffectInstance,
    report: ImportReport,
}

fn run(node: &AeProp) -> Run {
    let mut report = ImportReport::default();
    let tb = TimeBase::fallback();
    let ids: BTreeMap<u32, Uuid> = (1..=3).map(|i| (i, layer_id(i))).collect();
    let inst = {
        let mut conv = Conv {
            report: &mut report,
            tb,
            offset: Rational::ZERO,
            size: (W, H),
            solids: false,
            span: (Rational::ZERO, tb.seconds(2.0)),
            layer_ids: ids,
            masks: Vec::new(),
            // The composition's first layer is the one the effect is on,
            // which is what "this layer" means to a layer-reference row.
            self_index: 1,
        };
        let path = ItemPath::item("Comp 1").layer("Layer 1");
        claim(&mut conv, &path, node).expect("the table claims this match name")
    };
    Run { inst, report }
}

// ── reading the answer ──

fn f(r: &Run, id: &str) -> f64 {
    r.inst.float_at(id, 0.0).expect("a float parameter")
}

fn choice(r: &Run, id: &str) -> u32 {
    match r.inst.param(id) {
        Some(EffectValue::Choice(v)) => *v,
        other => panic!("{id} is not a choice: {other:?}"),
    }
}

fn boolean(r: &Run, id: &str) -> bool {
    match r.inst.param(id) {
        Some(EffectValue::Bool(v)) => *v,
        other => panic!("{id} is not a switch: {other:?}"),
    }
}

fn layer_of(r: &Run, id: &str) -> Option<Uuid> {
    match r.inst.param(id) {
        Some(EffectValue::Layer(v)) => *v,
        other => panic!("{id} is not a layer reference: {other:?}"),
    }
}

/// Every key of an animated parameter as (seconds, value, out-handle speed).
fn keys_of(r: &Run, id: &str) -> Vec<(f64, f64, f64)> {
    let Some(EffectValue::Float(p)) = r.inst.param(id) else {
        panic!("{id} is not a float");
    };
    let Animation::Keyframed(keys) = &p.animation else {
        panic!("{id} is not animated: {:?}", p.animation);
    };
    keys.iter()
        .map(|k| {
            let speed = match k.interp_out {
                SideInterp::Bezier { speed, .. } => speed,
                _ => 0.0,
            };
            (k.time.to_f64(), k.value, speed)
        })
        .collect()
}

fn rebased(r: &Run, param: &str) -> bool {
    r.report
        .rows
        .iter()
        .any(|row| matches!(&row.reason, Reason::EffectParamRebased { param: p, .. } if p == param))
}

fn approximated(r: &Run, param: &str) -> bool {
    r.report.rows.iter().any(
        |row| matches!(&row.reason, Reason::EffectParamApproximated { param: p, .. } if p == param),
    )
}

// ───────────────────────── the rows ─────────────────────────

#[test]
fn motion_tile_converts_its_four_sizes_and_carries_the_rest() {
    let ae = "ADBE Tile";
    let r = run(&fx(
        ae,
        "Motion Tile",
        vec![
            point(ae, 1, "Tile Center", 480.0, 270.0),
            keyed(
                ae,
                2,
                "Tile Width",
                &[(0.0, 100.0, 0.0), (2.0, 25.0, -50.0)],
            ),
            num(ae, 3, "Tile Height", 40.0),
            num(ae, 4, "Output Width", 150.0),
            num(ae, 5, "Output Height", 120.0),
            num(ae, 6, "Mirror Edges", 1.0),
            num(ae, 7, "Phase", 180.0),
            num(ae, 8, "Horizontal Phase Shift", 1.0),
        ],
    ));
    assert_eq!(
        (f(&r, "tile_centre_x"), f(&r, "tile_centre_y")),
        (480.0, 270.0)
    );
    // AE keeps the four sizes as per cents of the frame and Lumit as
    // px@comp, so each axis converts against the comp's own extent
    // — keyframes and their speeds with it.
    assert_eq!(
        keys_of(&r, "tile_width"),
        vec![(0.0, 1.00 * W, 0.0), (2.0, 0.25 * W, -0.50 * W)]
    );
    assert_eq!(f(&r, "tile_height"), 0.40 * H);
    assert_eq!(
        (f(&r, "output_width"), f(&r, "output_height")),
        (1.50 * W, 1.20 * H)
    );
    assert!(boolean(&r, "mirror_edges") && boolean(&r, "horizontal_phase_shift"));
    assert_eq!(f(&r, "phase"), 180.0);
    // The four conversions are the only rows: a number whose base changed
    // is reported, and everything else here is one for one.
    for name in ["Tile Width", "Tile Height", "Output Width", "Output Height"] {
        assert!(rebased(&r, name), "{name} changed base and must say so");
    }
    assert_eq!(r.report.rows.len(), 4);
}

/// **Set channels: four After Effects source layers onto one Source row.**
///
/// The three cases that actually occur in the reference project, in one
/// instance each. Every channel that names the chosen source layer, or the
/// layer the effect is on, converts exactly; a *second* source layer and
/// After Effects' "None" are both reported and left at the identity, which
/// is §5's "reported rather than approximated".
#[test]
fn set_channels_folds_four_source_layers_onto_one_source_row() {
    let ae = "ADBE Set Channels";
    // The commonest shape by far: all four channels off one other layer,
    // the identity assignment. Six of the project's ten instances.
    let r = run(&fx(
        ae,
        "Set Channels",
        vec![
            layer_ref(ae, 1, "Source Layer 1", 2),
            num(ae, 2, "Set Red To Source 1's", 1.0),
            layer_ref(ae, 3, "Source Layer 2", 2),
            num(ae, 4, "Set Green To Source 2's", 2.0),
            layer_ref(ae, 5, "Source Layer 3", 2),
            num(ae, 6, "Set Blue To Source 3's", 3.0),
            layer_ref(ae, 7, "Source Layer 4", 2),
            num(ae, 8, "Set Alpha To Source 4's", 4.0),
        ],
    ));
    assert_eq!(layer_of(&r, "source"), Some(layer_id(2)));
    assert_eq!(choice(&r, "red_from"), 5, "Source red");
    assert_eq!(choice(&r, "green_from"), 6, "Source green");
    assert_eq!(choice(&r, "blue_from"), 7, "Source blue");
    assert_eq!(choice(&r, "alpha_from"), 8, "Source alpha");

    // Three channels off another layer, the alpha off **this** one, which
    // is After Effects' own default for a picker nobody touched. This
    // layer's channels are the first five options, so it needs no Source
    // row and no report.
    let r = run(&fx(
        ae,
        "Set Channels",
        vec![
            layer_ref(ae, 1, "Source Layer 1", 3),
            num(ae, 2, "Set Red To Source 1's", 1.0),
            layer_ref(ae, 3, "Source Layer 2", 3),
            num(ae, 4, "Set Green To Source 2's", 2.0),
            layer_ref(ae, 5, "Source Layer 3", 3),
            num(ae, 6, "Set Blue To Source 3's", 3.0),
            layer_ref(ae, 7, "Source Layer 4", 1),
            num(ae, 8, "Set Alpha To Source 4's", 5.0),
        ],
    ));
    assert_eq!(layer_of(&r, "source"), Some(layer_id(3)));
    assert_eq!(choice(&r, "red_from"), 5, "Source red");
    assert_eq!(choice(&r, "alpha_from"), 4, "this layer's own luminance");
    assert!(
        !approximated(&r, "Source Layer 4"),
        "naming the layer the effect is on is exact, not an approximation"
    );

    // **An absent picker is the layer the effect is on**, not None: the
    // file stores only what is not at its default, and that is the default.
    let r = run(&fx(
        ae,
        "Set Channels",
        vec![num(ae, 8, "Set Alpha To Source 4's", 5.0)],
    ));
    assert_eq!(layer_of(&r, "source"), None, "nothing named a second layer");
    assert_eq!(choice(&r, "alpha_from"), 4, "this layer's own luminance");
    assert!(!approximated(&r, "Source Layer 4"));

    // A second source layer, After Effects' None, and one of the three
    // channels Lumit does not have. All three are reported; none of them
    // guesses at a picture.
    let r = run(&fx(
        ae,
        "Set Channels",
        vec![
            layer_ref(ae, 1, "Source Layer 1", 2),
            num(ae, 2, "Set Red To Source 1's", 9.0),
            layer_ref(ae, 3, "Source Layer 2", 3),
            num(ae, 4, "Set Green To Source 2's", 2.0),
            layer_ref(ae, 5, "Source Layer 3", 0),
            num(ae, 6, "Set Blue To Source 3's", 3.0),
            layer_ref(ae, 7, "Source Layer 4", 2),
            num(ae, 8, "Set Alpha To Source 4's", 7.0),
        ],
    ));
    assert_eq!(layer_of(&r, "source"), Some(layer_id(2)));
    assert_eq!(choice(&r, "red_from"), 10, "AE's Full On");
    assert_eq!(choice(&r, "green_from"), 1, "the second source is refused");
    assert!(
        approximated(&r, "Source Layer 2"),
        "the picker is what could not be carried, so the picker is what the report names"
    );
    assert_eq!(choice(&r, "blue_from"), 2, "None is refused");
    assert!(approximated(&r, "Source Layer 3"));
    assert_eq!(
        choice(&r, "alpha_from"),
        9,
        "Lightness collapses onto the chosen source's luminance"
    );
    assert!(approximated(&r, "Set Alpha To Source 4's"));
}

/// Wave Speed carries onto Lumit's own Wave speed, and Ripple Phase onto
/// Evolution, as they are.
#[test]
fn ripple_rebases_its_three_lengths_and_carries_wave_speed() {
    let ae = "ADBE Ripple";
    let r = run(&fx(
        ae,
        "Ripple",
        vec![
            num(ae, 1, "Radius", 50.0),
            point(ae, 2, "Center of Ripple", 700.0, 400.0),
            num(ae, 3, "Type of Conversion", 2.0),
            num(ae, 4, "Wave Speed", 1.5),
            keyed(ae, 5, "Wave Width", &[(0.0, 20.0, 0.0), (2.0, 60.0, 20.0)]),
            num(ae, 6, "Wave Height", 40.0),
            num(ae, 7, "Ripple Phase", 90.0),
        ],
    ));
    assert_eq!(f(&r, "radius"), 50.0 * (diag() / 2.0) / 100.0);
    assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (700.0, 400.0));
    assert_eq!(choice(&r, "wave_type"), 0, "AE's second entry is Symmetric");
    assert_eq!(keys_of(&r, "wave_width")[1], (2.0, 60.0, 20.0));
    assert_eq!(f(&r, "wave_height"), 40.0);
    assert_eq!(f(&r, "wave_speed"), 1.5);
    assert_eq!(f(&r, "evolution"), 90.0);
}

/// The five Expression Controls carry their one property, keyframes and
/// all, and report nothing — there is nothing to convert. The
/// keyframed slider is the case that matters: a CC-pack rig is an animated
/// Slider Control and a page of expressions reading it.
#[test]
fn the_expression_controls_carry_their_one_property() {
    let r = run(&fx(
        "ADBE Slider Control",
        "Slider Control",
        vec![keyed(
            "ADBE Slider Control",
            1,
            "Slider",
            &[(0.0, 0.0, 0.0), (2.0, 240.0, 120.0)],
        )],
    ));
    assert_eq!(keys_of(&r, "slider")[1], (2.0, 240.0, 120.0));
    assert!(r.report.rows.is_empty(), "nothing to report");

    let r = run(&fx(
        "ADBE Angle Control",
        "Angle Control",
        vec![num("ADBE Angle Control", 1, "Angle", 450.0)],
    ));
    assert_eq!(f(&r, "angle"), 450.0, "an angle winds past a full turn");

    let r = run(&fx(
        "ADBE Checkbox Control",
        "Checkbox Control",
        vec![num("ADBE Checkbox Control", 1, "Checkbox", 1.0)],
    ));
    assert!(boolean(&r, "checkbox"));

    let r = run(&fx(
        "ADBE Color Control",
        "Color Control",
        vec![colour(
            "ADBE Color Control",
            1,
            "Color",
            [0.5, 0.5, 0.5, 1.0],
        )],
    ));
    match r.inst.param("colour") {
        Some(EffectValue::Colour(c)) => {
            let red = c[0].value_at(0.0);
            assert!(
                (red - f64::from(srgb_to_linear(0.5))).abs() < 1e-9,
                "the swatch crosses into scene-linear light like every other                      imported colour: {red}"
            );
        }
        other => panic!("not a colour: {other:?}"),
    }

    let r = run(&fx(
        "ADBE Point Control",
        "Point Control",
        vec![point("ADBE Point Control", 1, "Point", 640.0, 360.0)],
    ));
    assert_eq!((f(&r, "point_x"), f(&r, "point_y")), (640.0, 360.0));
}

/// A conversion cannot rewrite an expression, so a rebased parameter with
/// one on it is reported rather than quietly wrong.
#[test]
fn an_expression_under_a_rebased_parameter_is_reported() {
    let ae = "ADBE Twirl";
    let mut node = num(ae, 2, "Twirl Radius", 0.0);
    node.expression = Some("time * 10".to_string());
    node.expression_enabled = Some(true);
    let r = run(&fx(ae, "Twirl", vec![node]));
    assert!(approximated(&r, "Twirl Radius"));
}
