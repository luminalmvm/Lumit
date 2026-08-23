//! Per-effect conversion tests for the colour / blur / generate / temporal
//! half of the table.
//!
//! Every test feeds one synthetic captured instance with its values off their
//! After Effects defaults and at least one parameter keyframed, then asserts
//! the Lumit instance parameter for parameter and the report rows docs/11 §5's
//! row promised. Unit conversions are computed here from the composition size
//! rather than copied, so a comp of another shape would fail a wrong constant.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use lumit_core::anim::{Animation, Property as LumProperty, SideInterp};
use lumit_core::mask::{BezierPath, Vertex};
use lumit_core::model::{EffectInstance, EffectNamespace, EffectValue};
use lumit_core::time::Rational;
use uuid::Uuid;

use super::*;
use crate::capture::{Ease, Keyframe as AeKey, Property as AeProp};
use crate::map::time::TimeBase;
use crate::report::{ImportReport, Reason};

/// The composition every test converts against.
const W: f64 = 1920.0;
const H: f64 = 1080.0;

// --- building a capture ----------------------------------------------------

fn leaf(match_name: &str, value: serde_json::Value) -> AeProp {
    AeProp {
        match_name: Some(match_name.to_string()),
        name: Some(match_name.to_string()),
        value_type: Some("float".to_string()),
        value: Some(value),
        ..AeProp::default()
    }
}

/// A keyframed leaf: `(seconds, value, out speed)` per key, both sides bezier,
/// so the test can watch a handle's speed cross into the new units with the
/// value.
fn keyed(match_name: &str, keys: &[(f64, f64, f64)]) -> AeProp {
    AeProp {
        match_name: Some(match_name.to_string()),
        name: Some(match_name.to_string()),
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

fn effect(match_name: &str, name: &str, params: Vec<AeProp>) -> AeProp {
    AeProp {
        match_name: Some(match_name.to_string()),
        name: Some(name.to_string()),
        enabled: Some(true),
        group: Some(params),
        ..AeProp::default()
    }
}

// --- running it ------------------------------------------------------------

struct Ran {
    inst: EffectInstance,
    report: ImportReport,
    mapped: bool,
}

fn run(node: &AeProp) -> Ran {
    run_with_masks(node, Vec::new())
}

fn run_with_masks(node: &AeProp, masks: Vec<(Uuid, f64)>) -> Ran {
    let mut report = ImportReport::default();
    let mapped;
    let inst = {
        let mut conv = Conv {
            report: &mut report,
            tb: TimeBase::of_fps(Some(25.0)).expect("a rate"),
            offset: Rational::ZERO,
            size: (W, H),
            span: (Rational::ZERO, Rational::new(4, 1).unwrap()),
            layer_ids: BTreeMap::new(),
            masks,
        };
        let path = crate::report::ItemPath::item("Comp").layer("Layer");
        let out = crate::map::map_effect(&mut conv, &path, node);
        mapped = matches!(out, crate::map::MappedEffect::Mapped(_));
        out.instance()
    };
    Ran {
        inst,
        report,
        mapped,
    }
}

impl Ran {
    fn prop(&self, id: &str) -> &LumProperty {
        match self.inst.param(id) {
            Some(EffectValue::Float(p)) => p,
            other => panic!("{id} is {other:?}, not a float"),
        }
    }

    fn f(&self, id: &str) -> f64 {
        match &self.prop(id).animation {
            Animation::Static(v) => *v,
            other => panic!("{id} is {other:?}, not a still value"),
        }
    }

    fn keys(&self, id: &str) -> Vec<lumit_core::anim::Keyframe> {
        match &self.prop(id).animation {
            Animation::Keyframed(k) => k.clone(),
            other => panic!("{id} is {other:?}, not keyframed"),
        }
    }

    fn choice(&self, id: &str) -> u32 {
        match self.inst.param(id) {
            Some(EffectValue::Choice(v)) => *v,
            other => panic!("{id} is {other:?}, not a choice"),
        }
    }

    fn flag(&self, id: &str) -> bool {
        match self.inst.param(id) {
            Some(EffectValue::Bool(v)) => *v,
            other => panic!("{id} is {other:?}, not a switch"),
        }
    }

    fn seed(&self, id: &str) -> u32 {
        match self.inst.param(id) {
            Some(EffectValue::Seed(v)) => *v,
            other => panic!("{id} is {other:?}, not a seed"),
        }
    }

    fn mask(&self, id: &str) -> Option<Uuid> {
        match self.inst.param(id) {
            Some(EffectValue::MaskPath(v)) => *v,
            other => panic!("{id} is {other:?}, not a mask-path row"),
        }
    }

    fn colour(&self, id: &str) -> [f64; 4] {
        match self.inst.param(id) {
            Some(EffectValue::Colour(c)) => [
                c[0].value_at(0.0),
                c[1].value_at(0.0),
                c[2].value_at(0.0),
                c[3].value_at(0.0),
            ],
            other => panic!("{id} is {other:?}, not a colour"),
        }
    }

    fn dropped(&self, param: &str) -> bool {
        self.report.rows.iter().any(
            |r| matches!(&r.reason, Reason::EffectParamNotCarried { param: p, .. } if p == param),
        )
    }

    fn approximated(&self, param: &str) -> bool {
        self.report.rows.iter().any(
            |r| matches!(&r.reason, Reason::EffectParamApproximated { param: p, .. } if p == param),
        )
    }

    fn rebased(&self, param: &str) -> bool {
        self.report
            .rows
            .iter()
            .any(|r| matches!(&r.reason, Reason::EffectParamRebased { param: p, .. } if p == param))
    }

    fn differs(&self) -> bool {
        self.report
            .rows
            .iter()
            .any(|r| matches!(r.reason, Reason::EffectDiffers { .. }))
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn linear(v: f64) -> f64 {
    f64::from(srgb_to_linear(v))
}

// ---------------------------------------------------------------------------
// Blur and sharpen
// ---------------------------------------------------------------------------

/// **A blur radius carries as pixels — on the still value, on every key, and
/// on the handles.** After Effects' raster pixels are Lumit's px@comp (docs/08
/// §2.3, K-419): the number is the same, and Lumit's preview scaling is what
/// keeps a Half preview looking like the export.
#[test]
fn gaussian_blur_carries_its_radius_and_its_keyframes() {
    let ran = run(&effect(
        "ADBE Gaussian Blur 2",
        "Gaussian Blur",
        vec![
            keyed(
                "ADBE Gaussian Blur 2-0001",
                &[(0.0, 22.0, 5.0), (2.0, 88.0, 5.0)],
            ),
            leaf("ADBE Gaussian Blur 2-0002", serde_json::json!(2)),
            leaf("ADBE Gaussian Blur 2-0003", serde_json::json!(1)),
        ],
    ));

    assert!(ran.mapped);
    assert_eq!(ran.inst.effect.match_name, "blur");
    assert_eq!(ran.inst.effect.namespace, EffectNamespace::Builtin);

    let keys = ran.keys("radius");
    assert_eq!(keys.len(), 2);
    assert!(close(keys[0].value, 22.0));
    assert!(close(keys[1].value, 88.0));
    assert!(matches!(
        keys[0].interp_out,
        SideInterp::Bezier { speed, influence }
            if close(speed, 5.0) && close(influence, 0.5)
    ));

    assert!(
        !ran.rebased("Blurriness"),
        "pixels are pixels: nothing to report"
    );
    assert!(ran.dropped("Blur Dimensions"));
    assert!(ran.dropped("Repeat Edge Pixels"));
}

/// **Directional blur's angle and length both carry unchanged.** docs/11 §5:
/// the angle is degrees from straight up clockwise on both sides and the
/// length is pixels on both, so converting either would be the error.
#[test]
fn directional_blur_keeps_the_angle_and_the_length() {
    let ran = run(&effect(
        "ADBE Motion Blur",
        "Directional Blur",
        vec![
            leaf("ADBE Motion Blur-0001", serde_json::json!(37.5)),
            keyed("ADBE Motion Blur-0002", &[(0.0, 64.0, 0.0)]),
        ],
    ));

    assert_eq!(ran.inst.effect.match_name, "directional_blur");
    assert!(close(ran.f("angle"), 37.5));
    assert!(close(ran.keys("length")[0].value, 64.0));
    assert!(!ran.rebased("Blur Length"));
}

/// **Radial blur's centre is a point in After Effects and a per cent of the
/// frame in Lumit, so each axis divides through its own dimension.**
#[test]
fn radial_blur_converts_its_centre_per_axis() {
    let mut centre = leaf("ADBE Radial Blur-0002", serde_json::json!([480.0, 810.0]));
    centre.value_type = Some("point".to_string());
    let ran = run(&effect(
        "ADBE Radial Blur",
        "Radial Blur",
        vec![
            keyed("ADBE Radial Blur-0001", &[(0.0, 30.0, 0.0)]),
            centre,
            leaf("ADBE Radial Blur-0003", serde_json::json!(2)),
            leaf("ADBE Radial Blur-0004", serde_json::json!(1)),
            leaf("ADBE Radial Blur-0006", serde_json::json!(7)),
        ],
    ));

    assert_eq!(ran.inst.effect.match_name, "radial_blur");
    assert!(close(ran.keys("amount")[0].value, 30.0));
    assert!(close(ran.f("centre_x"), 25.0));
    assert!(close(ran.f("centre_y"), 75.0));
    // AE's Type 2 is Zoom, Lumit's index 1.
    assert_eq!(ran.choice("radial_type"), 1);
    assert!(ran.dropped("Antialiasing (Best Quality)"));
    assert!(ran.dropped("Random Seed"));
}

/// **Glow's threshold is an eight-bit display value and Lumit's is light**, so
/// it crosses the sRGB transfer function; the radius and intensity carry.
#[test]
fn glow_converts_its_threshold_into_light() {
    let ran = run(&effect(
        "ADBE Glo2",
        "Glow",
        vec![
            leaf("ADBE Glo2-0001", serde_json::json!(1)),
            leaf("ADBE Glo2-0002", serde_json::json!(191.25)),
            keyed("ADBE Glo2-0003", &[(0.0, 40.0, 0.0)]),
            leaf("ADBE Glo2-0004", serde_json::json!(2.5)),
        ],
    ));

    assert_eq!(ran.inst.effect.match_name, "glow");
    assert!(close(ran.f("threshold"), linear(0.75)));
    assert!(close(ran.keys("radius")[0].value, 40.0));
    assert!(close(ran.f("intensity"), 2.5));
    assert!(ran.differs());
    assert!(ran.dropped("the Glow Colors group"));
}

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// **Levels writes the group its Channel picker named.** The scripting DOM
/// exposes one set of five numbers, so a green-channel grade would land on the
/// master lane if the picker were ignored — and the master lane is the one
/// thing it must not touch.
#[test]
fn levels_writes_the_channel_its_picker_named() {
    let ran = run(&effect(
        "ADBE Easy Levels2",
        "Levels",
        vec![
            leaf("ADBE Easy Levels2-0001", serde_json::json!(3)),
            keyed("ADBE Easy Levels2-0003", &[(0.0, 0.1, 0.0)]),
            leaf("ADBE Easy Levels2-0004", serde_json::json!(0.9)),
            leaf("ADBE Easy Levels2-0005", serde_json::json!(1.4)),
            leaf("ADBE Easy Levels2-0006", serde_json::json!(0.05)),
            leaf("ADBE Easy Levels2-0007", serde_json::json!(0.95)),
        ],
    ));

    assert_eq!(ran.inst.effect.match_name, "levels");
    assert!(close(ran.keys("green_in_black")[0].value, 0.1));
    assert!(close(ran.f("green_in_white"), 0.9));
    assert!(close(ran.f("green_gamma"), 1.4));
    assert!(close(ran.f("green_out_black"), 0.05));
    assert!(close(ran.f("green_out_white"), 0.95));
    // The master lane is left neutral.
    assert!(close(ran.f("master_in_white"), 1.0));
    assert!(ran.differs());
    assert!(ran.dropped("Clip To Output Black"));
}

/// **Hue/Saturation writes the range its picker named, and a colourised
/// instance takes the placeholder road** rather than being approximated —
/// Colorize discards the source hue and nothing in Lumit does that.
#[test]
fn hue_saturation_writes_one_range_and_refuses_colorize() {
    let ran = run(&effect(
        "ADBE HUE SATURATION",
        "Hue/Saturation",
        vec![
            leaf("ADBE HUE SATURATION-0002", serde_json::json!(5)),
            keyed("ADBE HUE SATURATION-0004", &[(0.0, 24.0, 0.0)]),
            leaf("ADBE HUE SATURATION-0005", serde_json::json!(-30.0)),
            leaf("ADBE HUE SATURATION-0006", serde_json::json!(12.0)),
            leaf("ADBE HUE SATURATION-0007", serde_json::json!(0)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "hue_saturation");
    assert!(close(ran.keys("cyans_hue")[0].value, 24.0));
    assert!(close(ran.f("cyans_saturation"), -30.0));
    assert!(close(ran.f("cyans_lightness"), 12.0));
    assert!(close(ran.f("master_saturation"), 0.0));

    let colourised = run(&effect(
        "ADBE HUE SATURATION",
        "Hue/Saturation",
        vec![
            leaf("ADBE HUE SATURATION-0002", serde_json::json!(1)),
            leaf("ADBE HUE SATURATION-0007", serde_json::json!(1)),
        ],
    ));
    assert!(!colourised.mapped);
    assert_eq!(
        colourised.inst.effect.namespace,
        EffectNamespace::Placeholder
    );
    assert!(colourised.dropped("Colorize"));
}

/// **Brightness & Contrast is one Lumit effect carrying both sliders under
/// AE's names and AE's neutral point** (K-397), so both numbers cross
/// unchanged.
#[test]
fn brightness_and_contrast_carry_both_sliders_unchanged() {
    let ran = run(&effect(
        "ADBE Brightness & Contrast 2",
        "Brightness & Contrast",
        vec![
            keyed("ADBE Brightness & Contrast 2-0001", &[(0.0, -18.0, 2.0)]),
            leaf("ADBE Brightness & Contrast 2-0002", serde_json::json!(44.0)),
            leaf("ADBE Brightness & Contrast 2-0003", serde_json::json!(1)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "brightness");
    assert!(close(ran.keys("brightness")[0].value, -18.0));
    assert!(close(ran.f("contrast"), 44.0));
    assert!(ran.dropped("Use Legacy (supports HDR)"));
}

/// **Tint's two colours cross into scene-linear light and its Amount to Tint
/// is Lumit's Mix**, both per cents of the same thing.
#[test]
fn tint_converts_both_colours_and_its_amount() {
    let mut black = leaf("ADBE Tint-0001", serde_json::json!([0.2, 0.1, 0.05, 1.0]));
    black.value_type = Some("colour".to_string());
    let mut white = leaf("ADBE Tint-0002", serde_json::json!([0.9, 0.8, 0.7, 1.0]));
    white.value_type = Some("colour".to_string());
    let ran = run(&effect(
        "ADBE Tint",
        "Tint",
        vec![black, white, keyed("ADBE Tint-0003", &[(0.0, 65.0, 0.0)])],
    ));

    assert_eq!(ran.inst.effect.match_name, "tint");
    let b = ran.colour("black");
    assert!(close(b[0], linear(0.2)) && close(b[1], linear(0.1)) && close(b[3], 1.0));
    assert!(close(ran.colour("white")[2], linear(0.7)));
    assert!(close(ran.keys("mix")[0].value, 65.0));
}

/// **Photo filter's twenty-one entries are the same twenty-one, one index
/// apart**, and the named filters are a look-for-look conversion.
#[test]
fn photo_filter_maps_the_filter_list_by_position() {
    let mut colour = leaf(
        "ADBE Photo Filter-0002",
        serde_json::json!([0.9, 0.5, 0.1, 1.0]),
    );
    colour.value_type = Some("colour".to_string());
    let ran = run(&effect(
        "ADBE Photo Filter",
        "Photo Filter",
        vec![
            leaf("ADBE Photo Filter-0001", serde_json::json!(21)),
            colour,
            keyed("ADBE Photo Filter-0003", &[(0.0, 60.0, 0.0)]),
            leaf("ADBE Photo Filter-0004", serde_json::json!(0)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "photo_filter");
    // AE's twenty-first entry is Custom, Lumit's index 20.
    assert_eq!(ran.choice("filter"), 20);
    assert!(close(ran.colour("colour")[0], linear(0.9)));
    assert!(close(ran.keys("density")[0].value, 60.0));
    assert!(!ran.flag("preserve_luminosity"));
    assert!(ran.differs());
}

/// **Black & white's six weights carry one for one**, and the tint colour is
/// reported because the effect divides it through by its own luma.
#[test]
fn black_and_white_carries_six_weights_and_reports_the_tint() {
    let mut tint = leaf(
        "ADBE Black&White-0008",
        serde_json::json!([0.6, 0.4, 0.2, 1.0]),
    );
    tint.value_type = Some("colour".to_string());
    let ran = run(&effect(
        "ADBE Black&White",
        "Black & White",
        vec![
            keyed("ADBE Black&White-0001", &[(0.0, 55.0, 0.0)]),
            leaf("ADBE Black&White-0002", serde_json::json!(70.0)),
            leaf("ADBE Black&White-0003", serde_json::json!(35.0)),
            leaf("ADBE Black&White-0004", serde_json::json!(65.0)),
            leaf("ADBE Black&White-0005", serde_json::json!(15.0)),
            leaf("ADBE Black&White-0006", serde_json::json!(85.0)),
            leaf("ADBE Black&White-0007", serde_json::json!(1)),
            tint,
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "black_and_white");
    assert!(close(ran.keys("reds")[0].value, 55.0));
    assert!(close(ran.f("magentas"), 85.0));
    assert!(ran.flag("tint"));
    assert!(close(ran.colour("tint_colour")[1], linear(0.4)));
    assert!(ran.approximated("Tint Color"));
}

/// **Shadow/Highlight's two radii average into Lumit's one, px@comp**, and
/// Blend With Original inverts into Mix.
#[test]
fn shadow_highlight_averages_the_two_radii() {
    let ran = run(&effect(
        "ADBE ShadowHighlight",
        "Shadow/Highlight",
        vec![
            leaf("ADBE ShadowHighlight-0001", serde_json::json!(0)),
            keyed("ADBE ShadowHighlight-0002", &[(0.0, 60.0, 0.0)]),
            leaf("ADBE ShadowHighlight-0003", serde_json::json!(20.0)),
            leaf("ADBE ShadowHighlight-0007", serde_json::json!(40.0)),
            leaf("ADBE ShadowHighlight-0008", serde_json::json!(20.0)),
            leaf("ADBE ShadowHighlight-0009", serde_json::json!(55.0)),
            leaf("ADBE ShadowHighlight-0010", serde_json::json!(60.0)),
            leaf("ADBE ShadowHighlight-0011", serde_json::json!(35.0)),
            leaf("ADBE ShadowHighlight-0012", serde_json::json!(-10.0)),
            leaf("ADBE ShadowHighlight-0016", serde_json::json!(25.0)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "shadow_highlight");
    assert!(close(ran.keys("shadow_amount")[0].value, 60.0));
    assert!(close(ran.f("highlight_tonal_width"), 55.0));
    assert!(close(ran.f("radius"), 40.0));
    assert!(close(ran.f("midtone_contrast"), -10.0));
    assert!(close(ran.f("mix"), 75.0));
    assert!(ran.approximated("Shadow Radius and Highlight Radius"));
    assert!(ran.dropped("Scene Detect"));
    assert!(!ran.approximated("Auto Amounts"));
}

/// **Tritone's three stops cross into light and Blend With Original inverts
/// into Mix.**
#[test]
fn tritone_converts_three_stops_and_the_blend() {
    let stop = |mn: &str, v: f64| {
        let mut p = leaf(mn, serde_json::json!([v, v, v, 1.0]));
        p.value_type = Some("colour".to_string());
        p
    };
    let ran = run(&effect(
        "ADBE Tritone",
        "Tritone",
        vec![
            stop("ADBE Tritone-0001", 0.9),
            stop("ADBE Tritone-0002", 0.5),
            stop("ADBE Tritone-0003", 0.1),
            keyed("ADBE Tritone-0004", &[(0.0, 40.0, 0.0)]),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "tritone");
    assert!(close(ran.colour("highlights")[0], linear(0.9)));
    assert!(close(ran.colour("shadows")[2], linear(0.1)));
    assert!(close(ran.keys("mix")[0].value, 60.0));
    assert!(ran.differs());
}

/// **Posterize's one number carries unchanged**, and the difference — where
/// the bands land — is a row rather than an arithmetic fudge.
#[test]
fn posterize_carries_its_level_and_reports_where_the_bands_land() {
    let ran = run(&effect(
        "ADBE Posterize",
        "Posterize",
        vec![keyed(
            "ADBE Posterize-0001",
            &[(0.0, 5.0, 0.0), (1.0, 20.0, 0.0)],
        )],
    ));
    assert_eq!(ran.inst.effect.match_name, "posterize");
    assert!(close(ran.keys("levels")[1].value, 20.0));
    assert!(ran.differs());
}

/// **Threshold's eight-bit level becomes a per cent of the same placement.**
#[test]
fn threshold_converts_its_level_out_of_eight_bits() {
    let ran = run(&effect(
        "ADBE Threshold",
        "Threshold",
        vec![keyed("ADBE Threshold-0001", &[(0.0, 51.0, 0.0)])],
    ));
    assert_eq!(ran.inst.effect.match_name, "threshold");
    assert!(close(ran.keys("level")[0].value, 51.0 * 100.0 / 255.0));
    assert!(close(ran.f("softness"), 0.0));
    assert!(ran.rebased("Level"));
}

/// **Broadcast Colors maps both dropdowns by position and keeps the IRE
/// number**, which is the same unit on both sides.
#[test]
fn broadcast_colours_maps_both_lists_and_keeps_the_ire() {
    let ran = run(&effect(
        "ADBE Broadcast Colors",
        "Broadcast Colors",
        vec![
            leaf("ADBE Broadcast Colors-0001", serde_json::json!(2)),
            leaf("ADBE Broadcast Colors-0002", serde_json::json!(3)),
            keyed("ADBE Broadcast Colors-0003", &[(0.0, 100.0, 0.0)]),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "broadcast_safe");
    assert_eq!(ran.choice("standard"), 1);
    assert_eq!(ran.choice("how_to_treat"), 2);
    assert!(close(ran.keys("maximum_signal")[0].value, 100.0));
    assert!(ran.differs());
}

// ---------------------------------------------------------------------------
// Generate
// ---------------------------------------------------------------------------

/// **A whole-alpha Fill converts exactly and a mask-targeted one reports.**
/// AE's Opacity is Lumit's Mix, which is the same number a hundred times over.
#[test]
fn fill_maps_a_whole_alpha_fill_and_reports_a_mask_targeted_one() {
    let mut colour = leaf("ADBE Fill-0002", serde_json::json!([0.3, 0.6, 0.9, 1.0]));
    colour.value_type = Some("colour".to_string());
    let ran = run(&effect(
        "ADBE Fill",
        "Fill",
        vec![
            leaf("ADBE Fill-0001", serde_json::json!(0)),
            leaf("ADBE Fill-0007", serde_json::json!(0)),
            colour,
            keyed("ADBE Fill-0005", &[(0.0, 0.4, 0.0)]),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "fill");
    assert!(close(ran.colour("colour")[2], linear(0.9)));
    assert!(close(ran.keys("mix")[0].value, 40.0));
    assert!(!ran.approximated("Fill Mask"));

    let targeted = run(&effect(
        "ADBE Fill",
        "Fill",
        vec![
            leaf("ADBE Fill-0001", serde_json::json!(1)),
            leaf("ADBE Fill-0006", serde_json::json!(1)),
        ],
    ));
    assert!(targeted.approximated("Fill Mask"));
    assert!(targeted.dropped("Horizontal Feather"));
}

/// **Gradient Ramp's two points and two colours carry, and Blend With
/// Original inverts into Mix.**
#[test]
fn gradient_ramp_converts_both_ends_and_the_blend() {
    let point = |mn: &str, x: f64, y: f64| {
        let mut p = leaf(mn, serde_json::json!([x, y]));
        p.value_type = Some("point".to_string());
        p
    };
    let colour = |mn: &str, v: f64| {
        let mut p = leaf(mn, serde_json::json!([v, v, v, 1.0]));
        p.value_type = Some("colour".to_string());
        p
    };
    let ran = run(&effect(
        "ADBE Ramp",
        "Gradient Ramp",
        vec![
            point("ADBE Ramp-0001", 100.0, 200.0),
            colour("ADBE Ramp-0002", 0.8),
            point("ADBE Ramp-0003", 1500.0, 900.0),
            colour("ADBE Ramp-0004", 0.2),
            leaf("ADBE Ramp-0005", serde_json::json!(2)),
            keyed("ADBE Ramp-0006", &[(0.0, 30.0, 0.0)]),
            leaf("ADBE Ramp-0007", serde_json::json!(20.0)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "gradient");
    assert!(close(ran.f("start_x"), 100.0) && close(ran.f("start_y"), 200.0));
    assert!(close(ran.f("end_x"), 1500.0) && close(ran.f("end_y"), 900.0));
    assert!(close(ran.colour("start_colour")[0], linear(0.8)));
    assert_eq!(ran.choice("shape"), 1);
    assert!(close(ran.keys("scatter")[0].value, 30.0));
    assert!(close(ran.f("mix"), 80.0));
    assert!(ran.differs());
}

/// **Noise's amount and colour switch carry, and AE's clipping is a row.**
#[test]
fn noise_carries_its_amount_and_reports_the_clipping() {
    let ran = run(&effect(
        "ADBE Noise",
        "Noise",
        vec![
            keyed("ADBE Noise-0001", &[(0.0, 12.0, 1.0), (2.0, 48.0, 1.0)]),
            leaf("ADBE Noise-0002", serde_json::json!(1)),
            leaf("ADBE Noise-0003", serde_json::json!(1)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "noise");
    assert!(close(ran.keys("amount")[1].value, 48.0));
    assert!(ran.flag("colour_noise"));
    assert!(ran.dropped("Clipping"));
    assert!(ran.differs());
}

/// **Fractal noise's Scale converts through After Effects' own base into a
/// cell size in px@comp**, and AE's dozen fractal types collapse onto two with
/// a row saying so.
#[test]
fn fractal_noise_rebases_the_scale_and_collapses_the_type_lists() {
    let mut offset = leaf("ADBE Fractal Noise-0013", serde_json::json!([300.0, 400.0]));
    offset.value_type = Some("point".to_string());
    let ran = run(&effect(
        "ADBE Fractal Noise",
        "Fractal Noise",
        vec![
            leaf("ADBE Fractal Noise-0001", serde_json::json!(4)),
            leaf("ADBE Fractal Noise-0002", serde_json::json!(1)),
            leaf("ADBE Fractal Noise-0003", serde_json::json!(1)),
            leaf("ADBE Fractal Noise-0004", serde_json::json!(150.0)),
            leaf("ADBE Fractal Noise-0005", serde_json::json!(-25.0)),
            leaf("ADBE Fractal Noise-0006", serde_json::json!(2)),
            leaf("ADBE Fractal Noise-0008", serde_json::json!(45.0)),
            leaf("ADBE Fractal Noise-0009", serde_json::json!(0)),
            keyed("ADBE Fractal Noise-0010", &[(0.0, 75.0, 0.0)]),
            leaf("ADBE Fractal Noise-0011", serde_json::json!(120.0)),
            leaf("ADBE Fractal Noise-0012", serde_json::json!(80.0)),
            offset,
            leaf("ADBE Fractal Noise-0015", serde_json::json!(4)),
            leaf("ADBE Fractal Noise-0017", serde_json::json!(80.0)),
            leaf("ADBE Fractal Noise-0018", serde_json::json!(45.0)),
            leaf("ADBE Fractal Noise-0023", serde_json::json!(180.0)),
            leaf("ADBE Fractal Noise-0025", serde_json::json!(1)),
            leaf("ADBE Fractal Noise-0026", serde_json::json!(3)),
            leaf("ADBE Fractal Noise-0027", serde_json::json!(99)),
            leaf("ADBE Fractal Noise-0029", serde_json::json!(85.0)),
        ],
    ));

    assert_eq!(ran.inst.effect.match_name, "fractal_noise");
    assert_eq!(ran.choice("fractal_type"), 1);
    assert!(ran.approximated("ADBE Fractal Noise-0001"));
    // AE's Noise Type 1 is Block, which is Lumit's Value basis, exactly.
    assert_eq!(ran.choice("noise_type"), 0);
    assert!(ran.flag("invert"));
    assert!(close(ran.f("contrast"), 150.0));
    assert!(close(ran.f("brightness"), -25.0));
    assert!(close(ran.f("rotation"), 45.0));
    assert!(!ran.flag("uniform_scaling"));
    assert!(close(
        ran.keys("scale")[0].value,
        75.0 * AE_FRACTAL_SCALE_BASE
    ));
    assert!(close(ran.f("scale_width"), 120.0 * AE_FRACTAL_SCALE_BASE));
    assert!(close(ran.f("scale_height"), 80.0 * AE_FRACTAL_SCALE_BASE));
    assert!(close(ran.f("offset_x"), 300.0) && close(ran.f("offset_y"), 400.0));
    assert!(close(ran.f("complexity"), 4.0));
    assert!(close(ran.f("sub_influence"), 80.0));
    assert!(close(ran.f("sub_scaling"), 45.0));
    assert!(close(ran.f("evolution"), 180.0));
    assert!(ran.flag("cycle_evolution"));
    assert!(close(ran.f("cycle"), 3.0));
    assert_eq!(ran.seed("seed"), 99);
    assert!(close(ran.f("mix"), 85.0));
    assert!(ran.rebased("Scale"));
    assert!(ran.dropped("Overflow"));
    assert!(ran.dropped("Perspective Offset"));
}

/// **Beam's two points carry in pixels, its three per cents multiply by a
/// hundred, and 3D Perspective is a row** — it foreshortens from a camera
/// Lumit keeps on the composition.
#[test]
fn beam_converts_its_points_its_per_cents_and_reports_the_perspective() {
    let point = |mn: &str, x: f64, y: f64| {
        let mut p = leaf(mn, serde_json::json!([x, y]));
        p.value_type = Some("point".to_string());
        p
    };
    let colour = |mn: &str, v: f64| {
        let mut p = leaf(mn, serde_json::json!([v, v, v, 1.0]));
        p.value_type = Some("colour".to_string());
        p
    };
    let ran = run(&effect(
        "ADBE Laser",
        "Beam",
        vec![
            point("ADBE Laser-0001", 120.0, 300.0),
            point("ADBE Laser-0002", 1700.0, 800.0),
            keyed("ADBE Laser-0003", &[(0.0, 0.2, 0.1), (2.0, 0.8, 0.1)]),
            leaf("ADBE Laser-0004", serde_json::json!(0.35)),
            leaf("ADBE Laser-0005", serde_json::json!(18.0)),
            leaf("ADBE Laser-0006", serde_json::json!(4.0)),
            leaf("ADBE Laser-0007", serde_json::json!(0.6)),
            colour("ADBE Laser-0008", 0.95),
            colour("ADBE Laser-0009", 0.25),
            leaf("ADBE Laser-0010", serde_json::json!(1)),
            leaf("ADBE Laser-0011", serde_json::json!(0)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "beam");
    assert!(close(ran.f("start_x"), 120.0) && close(ran.f("end_y"), 800.0));
    let length = ran.keys("length");
    assert!(close(length[0].value, 20.0) && close(length[1].value, 80.0));
    // The handle's speed is in value-units a second, so it scales with them.
    assert!(matches!(
        length[0].interp_out,
        SideInterp::Bezier { speed, .. } if close(speed, 10.0)
    ));
    assert!(close(ran.f("time"), 35.0));
    assert!(close(ran.f("start_thickness"), 18.0));
    assert!(close(ran.f("softness"), 60.0));
    assert!(!ran.flag("composite_on_original"));
    assert!(ran.approximated("Softness"));
    assert!(ran.dropped("3D Perspective"));
}

/// **Advanced Lightning's four built types map exactly and the other four map
/// to the nearest, with a row.** Forking and Decay are AE's 0..1 fractions and
/// Lumit's per cents.
#[test]
fn lightning_maps_the_four_built_types_and_reports_the_rest() {
    let colour = |mn: &str, v: f64| {
        let mut p = leaf(mn, serde_json::json!([v, v, v, 1.0]));
        p.value_type = Some("colour".to_string());
        p
    };
    let point = |mn: &str, x: f64, y: f64| {
        let mut p = leaf(mn, serde_json::json!([x, y]));
        p.value_type = Some("point".to_string());
        p
    };
    let ran = run(&effect(
        "ADBE Lightning 2",
        "Advanced Lightning",
        vec![
            leaf("ADBE Lightning 2-0001", serde_json::json!(5)),
            point("ADBE Lightning 2-0002", 200.0, 150.0),
            point("ADBE Lightning 2-0003", 1600.0, 950.0),
            keyed("ADBE Lightning 2-0004", &[(0.0, 12.0, 0.0)]),
            leaf("ADBE Lightning 2-0006", serde_json::json!(5.0)),
            colour("ADBE Lightning 2-0008", 0.9),
            leaf("ADBE Lightning 2-0011", serde_json::json!(60.0)),
            leaf("ADBE Lightning 2-0012", serde_json::json!(40.0)),
            colour("ADBE Lightning 2-0013", 0.3),
            leaf("ADBE Lightning 2-0016", serde_json::json!(2.0)),
            leaf("ADBE Lightning 2-0017", serde_json::json!(0.4)),
            leaf("ADBE Lightning 2-0018", serde_json::json!(0.55)),
            leaf("ADBE Lightning 2-0020", serde_json::json!(1)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "lightning");
    // AE's type 5 is Omni, Lumit's index 2 — an exact entry, no row.
    assert_eq!(ran.choice("lightning_type"), 2);
    assert!(!ran.approximated("ADBE Lightning 2-0001"));
    assert!(close(ran.f("origin_x"), 200.0) && close(ran.f("direction_y"), 950.0));
    assert!(close(ran.keys("conductivity")[0].value, 12.0));
    assert!(close(ran.f("core_radius"), 5.0));
    assert!(close(ran.f("glow_opacity"), 40.0));
    assert!(close(
        ran.f("amplitude"),
        2.0 * AE_LIGHTNING_TURBULENCE_BASE
    ));
    assert!(close(ran.f("forking"), 40.0));
    assert!(close(ran.f("decay"), 55.0));
    assert!(ran.flag("composite_on_original"));
    assert!(ran.dropped("the Expert Settings group"));
    assert!(ran.dropped("Core Opacity"));

    let bouncey = run(&effect(
        "ADBE Lightning 2",
        "Advanced Lightning",
        vec![leaf("ADBE Lightning 2-0001", serde_json::json!(4))],
    ));
    assert_eq!(bouncey.choice("lightning_type"), 1);
    assert!(bouncey.approximated("ADBE Lightning 2-0001"));
}

/// **Radio Waves' clock becomes two Time keyframes running at one second a
/// second across the layer's own span**, and the two fade times become shares
/// of the lifespan.
#[test]
fn radio_waves_turns_the_clock_into_keyframes_and_the_fades_into_shares() {
    let mut producer = leaf("APC Radio Waves-0004", serde_json::json!([700.0, 500.0]));
    producer.value_type = Some("point".to_string());
    let mut colour = leaf(
        "APC Radio Waves-0046",
        serde_json::json!([0.1, 0.4, 0.8, 1.0]),
    );
    colour.value_type = Some("colour".to_string());
    let ran = run(&effect(
        "APC Radio Waves",
        "Radio Waves",
        vec![
            producer,
            leaf("APC Radio Waves-0002", serde_json::json!(1)),
            leaf("APC Radio Waves-0008", serde_json::json!(6)),
            leaf("APC Radio Waves-0014", serde_json::json!(1)),
            leaf("APC Radio Waves-0016", serde_json::json!(-0.4)),
            keyed("APC Radio Waves-0034", &[(0.0, 3.0, 0.0)]),
            leaf("APC Radio Waves-0036", serde_json::json!(120.0)),
            leaf("APC Radio Waves-0038", serde_json::json!(30.0)),
            leaf("APC Radio Waves-0044", serde_json::json!(90.0)),
            colour,
            leaf("APC Radio Waves-0050", serde_json::json!(7.0)),
            leaf("APC Radio Waves-0052", serde_json::json!(2.0)),
            leaf("APC Radio Waves-0054", serde_json::json!(0.8)),
            leaf("APC Radio Waves-0056", serde_json::json!(4.0)),
            leaf("APC Radio Waves-0058", serde_json::json!(1.0)),
            leaf("APC Radio Waves-0060", serde_json::json!(2.0)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "radio_waves");
    assert!(close(ran.f("centre_x"), 700.0) && close(ran.f("centre_y"), 500.0));

    // The layer runs four seconds, so Time runs 0 → 4 across it.
    let time = ran.keys("time");
    assert_eq!(time.len(), 2);
    assert!(close(time[0].value, 0.0) && close(time[1].value, 4.0));
    assert_eq!(time[1].time, Rational::new(4, 1).unwrap());

    assert!(close(ran.keys("frequency")[0].value, 3.0));
    assert!(close(ran.f("expansion"), 120.0));
    assert!(close(ran.f("rotation"), 30.0) && close(ran.f("spin"), 90.0));
    assert!(close(ran.f("lifespan"), 4.0));
    assert!(close(ran.f("sides"), 6.0));
    assert!(ran.flag("star"));
    assert!(close(ran.f("star_depth"), 40.0));
    assert!(close(ran.f("stroke_width"), 7.0));
    assert!(close(ran.f("opacity"), 80.0));
    // One second of a four-second lifespan is a quarter of it.
    assert!(close(ran.f("fade_in"), 25.0));
    assert!(close(ran.f("fade_out"), 50.0));
    assert!(ran.approximated("Star Depth"));
    assert!(ran.approximated("Start Width and End Width"));
    assert!(ran.dropped("Velocity"));
    assert!(ran
        .report
        .rows
        .iter()
        .any(|r| matches!(r.reason, Reason::EffectSpeedAsKeyframes { .. })));
}

/// **Vegas' count of segments becomes a length through the named mask's own
/// perimeter — exactly, on the half where a perimeter exists.** A 400 × 200
/// rectangle is 1200 units round; eight segments make each one 150 px@comp.
#[test]
fn vegas_divides_a_masks_perimeter_into_segment_lengths() {
    let mask = Uuid::now_v7();
    let mut colour = leaf("APC Vegas-0018", serde_json::json!([1.0, 0.9, 0.2, 1.0]));
    colour.value_type = Some("colour".to_string());
    let mut path = leaf("APC Vegas-0050", serde_json::json!(1));
    path.value_type = Some("mask".to_string());
    let ran = run_with_masks(
        &effect(
            "APC Vegas",
            "Vegas",
            vec![
                leaf("APC Vegas-0052", serde_json::json!(2)),
                path,
                leaf("APC Vegas-0028", serde_json::json!(8)),
                colour,
                keyed("APC Vegas-0020", &[(0.0, 6.0, 0.0)]),
                leaf("APC Vegas-0022", serde_json::json!(0.4)),
                leaf("APC Vegas-0024", serde_json::json!(0.75)),
                leaf("APC Vegas-0030", serde_json::json!(120.0)),
                leaf("APC Vegas-0036", serde_json::json!(0.9)),
            ],
        ),
        vec![(mask, 1200.0)],
    );

    assert_eq!(ran.inst.effect.match_name, "vegas");
    // Source's third entry is Mask/Path.
    assert_eq!(ran.choice("source"), 2);
    assert_eq!(ran.mask("path"), Some(mask));
    assert!(close(ran.f("segment_length"), 1200.0 / 8.0));
    assert!(!ran.approximated("Segments"));
    assert!(close(ran.colour("colour")[1], linear(0.9)));
    assert!(close(ran.keys("width")[0].value, 6.0));
    assert!(close(ran.f("hardness"), 40.0));
    assert!(close(ran.f("length"), 75.0));
    assert!(close(ran.f("rotation"), 120.0));
    assert!(close(ran.f("opacity"), 90.0));
    assert!(ran.dropped("Random Phase"));
}

/// **On the Image Contours half there is no perimeter to divide, so the
/// segment length is taken from the frame and the instance says so** — and
/// the eight-bit threshold becomes a per cent.
#[test]
fn vegas_on_image_contours_approximates_the_segment_length() {
    let ran = run(&effect(
        "APC Vegas",
        "Vegas",
        vec![
            leaf("APC Vegas-0052", serde_json::json!(1)),
            leaf("APC Vegas-0010", serde_json::json!(1)),
            leaf("APC Vegas-0012", serde_json::json!(51.0)),
            leaf("APC Vegas-0028", serde_json::json!(10)),
        ],
    ));
    assert_eq!(ran.choice("source"), 0);
    assert!(close(ran.f("threshold"), 51.0 * 100.0 / 255.0));
    assert!(close(ran.f("segment_length"), 2.0 * (W + H) / 10.0));
    assert!(ran.approximated("Segments"));
    assert!(ran.rebased("Threshold"));
    assert!(ran.differs());
}

/// **Add grain's multipliers become per cents on the neutral its own channel
/// balances pin, and a non-zero Animation Speed becomes the Animate switch.**
#[test]
fn add_grain_converts_multipliers_to_per_cents_and_speed_to_a_switch() {
    let ran = run(&effect(
        "VISINF Grain Implant",
        "Add Grain",
        vec![
            keyed("VISINF Grain Implant-0008", &[(0.0, 0.75, 0.0)]),
            leaf("VISINF Grain Implant-0007", serde_json::json!(2.5)),
            leaf("VISINF Grain Implant-0130", serde_json::json!(1.4)),
            leaf("VISINF Grain Implant-0002", serde_json::json!(1.2)),
            leaf("VISINF Grain Implant-0003", serde_json::json!(0.9)),
            leaf("VISINF Grain Implant-0004", serde_json::json!(1.1)),
            leaf("VISINF Grain Implant-0005", serde_json::json!(1)),
            leaf("VISINF Grain Implant-0040", serde_json::json!(0.5)),
            leaf("VISINF Grain Implant-0041", serde_json::json!(1.0)),
            leaf("VISINF Grain Implant-0042", serde_json::json!(1.5)),
            leaf("VISINF Grain Implant-0039", serde_json::json!(2.0)),
            leaf("VISINF Grain Implant-0013", serde_json::json!(42)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "add_grain");
    assert!(close(ran.keys("intensity")[0].value, 75.0));
    assert!(close(ran.f("size"), 2.5));
    assert!(close(ran.f("softness"), 70.0));
    assert!(close(ran.f("red"), 120.0) && close(ran.f("green"), 90.0));
    assert!(ran.flag("monochrome"));
    assert!(close(ran.f("shadows"), 50.0) && close(ran.f("highlights"), 150.0));
    assert!(ran.flag("animate"));
    assert_eq!(ran.seed("seed"), 42);
    assert!(ran.approximated("Animation Speed"));
    assert!(ran.rebased("Size"));
    assert!(ran.differs());
    assert!(ran.dropped("Preset"));

    let still = run(&effect(
        "VISINF Grain Implant",
        "Add Grain",
        vec![leaf("VISINF Grain Implant-0039", serde_json::json!(0.0))],
    ));
    assert!(!still.flag("animate"));
}

/// **Scribble carries the mask reference across (K-408)** and maps Wiggle Type
/// option for option — the one exact parity in the pair.
#[test]
fn scribble_carries_the_mask_and_the_wiggle_type() {
    let mask = Uuid::now_v7();
    let other = Uuid::now_v7();
    let mut path = leaf("ADBE Scribble Fill-0002", serde_json::json!(2));
    path.value_type = Some("mask".to_string());
    let mut colour = leaf(
        "ADBE Scribble Fill-0006",
        serde_json::json!([0.8, 0.2, 0.1, 1.0]),
    );
    colour.value_type = Some("colour".to_string());
    let ran = run_with_masks(
        &effect(
            "ADBE Scribble Fill",
            "Scribble",
            vec![
                leaf("ADBE Scribble Fill-0064", serde_json::json!(1)),
                path,
                leaf("ADBE Scribble Fill-0050", serde_json::json!(1)),
                colour,
                leaf("ADBE Scribble Fill-0024", serde_json::json!(0.6)),
                keyed("ADBE Scribble Fill-0010", &[(0.0, 75.0, 0.0)]),
                leaf("ADBE Scribble Fill-0008", serde_json::json!(3.5)),
                leaf("ADBE Scribble Fill-0060", serde_json::json!(9.0)),
                leaf("ADBE Scribble Fill-0038", serde_json::json!(-6.0)),
                leaf("ADBE Scribble Fill-0030", serde_json::json!(10.0)),
                leaf("ADBE Scribble Fill-0032", serde_json::json!(90.0)),
                leaf("ADBE Scribble Fill-0048", serde_json::json!(2)),
                leaf("ADBE Scribble Fill-0044", serde_json::json!(14.0)),
                leaf("ADBE Scribble Fill-0046", serde_json::json!(17)),
                leaf("ADBE Scribble Fill-0026", serde_json::json!(1)),
            ],
        ),
        vec![(other, 100.0), (mask, 200.0)],
    );
    assert_eq!(ran.inst.effect.match_name, "scribble");
    assert_eq!(ran.mask("path"), Some(mask));
    assert!(close(ran.colour("colour")[0], linear(0.8)));
    assert!(close(ran.f("opacity"), 60.0));
    assert!(close(ran.keys("angle")[0].value, 75.0));
    assert!(close(ran.f("stroke_width"), 3.5));
    assert!(close(ran.f("spacing"), 9.0));
    assert!(close(ran.f("path_overlap"), -6.0));
    assert!(close(ran.f("start"), 10.0) && close(ran.f("end"), 90.0));
    // AE's Wiggle Type 2 is Jagged, Lumit's index 1.
    assert_eq!(ran.choice("wiggle_type"), 1);
    assert!(close(ran.f("wiggles_per_second"), 14.0));
    assert_eq!(ran.seed("seed"), 17);
    assert!(!ran.flag("composite_on_original"));
    assert!(ran.rebased("Stroke Width, Spacing and Path Overlap"));
    assert!(ran.dropped("Curviness"));
}

/// **Stroke's brush size doubles, because After Effects' is a radius and
/// Lumit's is a width**, and All Masks imports pointed at the first mask.
#[test]
fn stroke_doubles_the_brush_and_reports_all_masks() {
    let mask = Uuid::now_v7();
    let mut path = leaf("ADBE Stroke-0001", serde_json::json!(0));
    path.value_type = Some("mask".to_string());
    let mut colour = leaf("ADBE Stroke-0002", serde_json::json!([0.2, 0.7, 0.4, 1.0]));
    colour.value_type = Some("colour".to_string());
    let ran = run_with_masks(
        &effect(
            "ADBE Stroke",
            "Stroke",
            vec![
                path,
                leaf("ADBE Stroke-0010", serde_json::json!(1)),
                leaf("ADBE Stroke-0011", serde_json::json!(1)),
                colour,
                keyed("ADBE Stroke-0003", &[(0.0, 6.0, 2.0)]),
                leaf("ADBE Stroke-0004", serde_json::json!(0.5)),
                leaf("ADBE Stroke-0005", serde_json::json!(0.85)),
                leaf("ADBE Stroke-0008", serde_json::json!(5.0)),
                leaf("ADBE Stroke-0009", serde_json::json!(70.0)),
                leaf("ADBE Stroke-0006", serde_json::json!(25.0)),
                leaf("ADBE Stroke-0007", serde_json::json!(3)),
            ],
        ),
        vec![(mask, 500.0)],
    );
    assert_eq!(ran.inst.effect.match_name, "stroke");
    // Unset in AE is Lumit's "First mask", which is the unset row.
    assert_eq!(ran.mask("path"), None);
    let brush = ran.keys("brush_size");
    assert!(close(brush[0].value, 12.0));
    assert!(matches!(
        brush[0].interp_out,
        SideInterp::Bezier { speed, .. } if close(speed, 4.0)
    ));
    assert!(close(ran.f("hardness"), 50.0));
    assert!(close(ran.f("opacity"), 85.0));
    assert!(close(ran.f("start"), 5.0) && close(ran.f("end"), 70.0));
    assert!(close(ran.f("spacing"), 25.0));
    // AE's Paint Style 3 is Reveal Original Image, Lumit's index 2.
    assert_eq!(ran.choice("paint_style"), 2);
    assert!(ran.approximated("All Masks"));
    assert!(ran.dropped("Stroke Sequentially"));
    assert!(ran.rebased("Brush Size"));
}

// ---------------------------------------------------------------------------
// Temporal
// ---------------------------------------------------------------------------

/// **Echo's count, decay and operator carry; its Echo Time is reported unless
/// it already names one frame back**, Lumit's samples being declared on whole
/// frames.
#[test]
fn echo_maps_its_operator_and_reports_a_sub_frame_echo_time() {
    let ran = run(&effect(
        "ADBE Echo",
        "Echo",
        vec![
            leaf("ADBE Echo-0001", serde_json::json!(-0.2)),
            keyed("ADBE Echo-0002", &[(0.0, 6.0, 0.0)]),
            leaf("ADBE Echo-0003", serde_json::json!(0.8)),
            leaf("ADBE Echo-0004", serde_json::json!(0.45)),
            leaf("ADBE Echo-0005", serde_json::json!(2)),
        ],
    ));
    assert_eq!(ran.inst.effect.match_name, "echo");
    assert!(close(ran.keys("echoes")[0].value, 6.0));
    assert!(close(ran.f("decay"), 0.45));
    // AE's Maximum is Lumit's Lighten, index 8.
    assert_eq!(ran.choice("mode"), 8);
    assert!(ran.approximated("Echo Time (seconds)"));
    assert!(ran.dropped("Starting Intensity"));

    // One frame back at 25 fps is exactly what Lumit does, so no row.
    let one_frame = run(&effect(
        "ADBE Echo",
        "Echo",
        vec![leaf("ADBE Echo-0001", serde_json::json!(-0.04))],
    ));
    assert!(!one_frame.approximated("Echo Time (seconds)"));
}

/// **Posterize time's one number means the same thing on both sides**, and
/// Lumit's extra Phase stays at the zero that is AE's behaviour.
#[test]
fn posterize_time_carries_its_rate_and_leaves_phase_alone() {
    let ran = run(&effect(
        "ADBE Posterize Time",
        "Posterize Time",
        vec![keyed("ADBE Posterize Time-0001", &[(0.0, 8.0, 0.0)])],
    ));
    assert_eq!(ran.inst.effect.match_name, "posterize_time");
    assert!(close(ran.keys("rate")[0].value, 8.0));
    assert!(close(ran.f("phase"), 0.0));
    // Nothing was adjusted, so nothing is reported.
    assert!(ran.report.rows.is_empty());
    assert_eq!(ran.report.imported, 2);
}

// ---------------------------------------------------------------------------
// The deliberate placeholders
// ---------------------------------------------------------------------------

/// **Curves imports as a placeholder because its point list is the one
/// property After Effects' own scripting cannot read** (K-410), and the report
/// says which property it was rather than leaving a silent gap.
#[test]
fn curves_imports_as_a_placeholder_with_its_unreadable_named() {
    let blob = AeProp {
        match_name: Some("ADBE CurvesCustom-0001".to_string()),
        name: Some("Curves".to_string()),
        value_type: Some("custom_blob".to_string()),
        unreadable: Some("Unable to execute script".to_string()),
        ..AeProp::default()
    };
    let ran = run(&effect(
        "ADBE CurvesCustom",
        "Curves",
        vec![leaf("ADBE CurvesCustom-0002", serde_json::json!(1)), blob],
    ));
    assert!(!ran.mapped);
    assert_eq!(ran.inst.effect.namespace, EffectNamespace::Placeholder);
    assert!(ran.report.rows.iter().any(
        |r| matches!(&r.reason, Reason::PropertyUnreadable { match_name }
            if match_name == "ADBE CurvesCustom-0001")
    ));
}

/// **Remove Grain and Timewarp are placeholders on purpose, and each says what
/// does the job instead** — a denoiser is a programme of its own, and a
/// retimer is Retime.
#[test]
fn the_two_deliberate_placeholders_name_what_to_use_instead() {
    for (match_name, name) in [
        ("VISINF Grain Removal", "Remove Grain"),
        ("ADBE Timewarp", "Timewarp"),
    ] {
        let ran = run(&effect(match_name, name, vec![]));
        assert!(!ran.mapped, "{match_name} should not map");
        assert_eq!(ran.inst.effect.namespace, EffectNamespace::Placeholder);
        assert!(
            ran.report.rows.iter().any(|r| matches!(
                &r.reason,
                Reason::EffectSuggestion { match_name: m, .. } if m == match_name
            )),
            "{match_name} should suggest what to use instead"
        );
    }
}

// ---------------------------------------------------------------------------
// The shared machinery
// ---------------------------------------------------------------------------

/// **An effect switched off in After Effects imports switched off**, and its
/// parameters still convert — a bypassed effect is not an absent one.
#[test]
fn an_effect_switched_off_imports_switched_off() {
    let mut node = effect(
        "ADBE Posterize",
        "Posterize",
        vec![leaf("ADBE Posterize-0001", serde_json::json!(4.0))],
    );
    node.enabled = Some(false);
    let ran = run(&node);
    assert!(!ran.inst.enabled);
    assert!(close(ran.f("levels"), 4.0));
}

/// **A parameter the walker could not read leaves the Lumit default in place
/// and the effect still maps.** One unreadable dial never costs the instance.
#[test]
fn a_missing_parameter_leaves_the_declared_default() {
    let ran = run(&effect("ADBE Gaussian Blur 2", "Gaussian Blur", vec![]));
    assert!(ran.mapped);
    // Gaussian blur's declared default radius (docs/08 §3.8).
    assert!(close(ran.f("radius"), 30.0));
}

/// **A mask reference naming a mask that did not come over falls back to the
/// first mask and says so** rather than pointing at nothing.
#[test]
fn a_mask_reference_that_missed_falls_back_and_reports() {
    let mut path = leaf("ADBE Stroke-0001", serde_json::json!(4));
    path.value_type = Some("mask".to_string());
    let ran = run(&effect("ADBE Stroke", "Stroke", vec![path]));
    assert_eq!(ran.mask("path"), None);
    assert!(ran.approximated("ADBE Stroke-0001"));
}

/// **A rectangle's perimeter is its four sides.** The estimate a curve needs
/// has to be exact on a polygon, because that is what most masks are — and it
/// is what turns Vegas' count of segments into a length.
#[test]
fn a_polygon_masks_perimeter_is_exact() {
    let corner = |x: f64, y: f64| Vertex {
        pos: (x, y),
        tan_in: (0.0, 0.0),
        tan_out: (0.0, 0.0),
    };
    let rect = BezierPath {
        vertices: vec![
            corner(0.0, 0.0),
            corner(400.0, 0.0),
            corner(400.0, 200.0),
            corner(0.0, 200.0),
        ],
        closed: true,
    };
    assert!(close(perimeter(&rect), 1200.0));

    // Open, so the closing side is not walked.
    let open = BezierPath {
        closed: false,
        ..rect
    };
    assert!(close(perimeter(&open), 1000.0));
}
