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
            self_index: 1,
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

// ---------------------------------------------------------------------------
// Blur and sharpen
// ---------------------------------------------------------------------------

/// **A blur radius carries as pixels — on the still value, on every key, and
/// on the handles.** After Effects' raster pixels are Lumit's px@comp (docs/08
/// §2.3): the number is the same, and Lumit's preview scaling is what
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

/// A keyframed dropdown imports at the value it starts on, and says so.
#[test]
fn a_keyframed_dropdown_imports_its_first_key_and_reports_it() {
    let ran = run(&effect(
        "ADBE Photo Filter",
        "Photo Filter",
        vec![keyed(
            "ADBE Photo Filter-0001",
            &[(0.0, 21.0, 0.0), (1.0, 2.0, 0.0)],
        )],
    ));
    assert_eq!(ran.choice("filter"), 20);
    assert!(ran.approximated("ADBE Photo Filter-0001"));
}

// ---------------------------------------------------------------------------
// Generate
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Temporal
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The deliberate placeholders
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Third-party effects: the two roads (docs/11 §5a)
// ---------------------------------------------------------------------------

/// A definition that arrived at run time, the way a scanned OFX plug-in's does.
/// Declared here rather than driven by a real bundle because what is under
/// test is the *importer's* half of the seam: given a catalogue entry under a
/// plug-in identifier, does an After Effects match name find it.
struct PluginDef(&'static lumit_core::fx::EffectSchema);

impl lumit_core::fx::EffectDef for PluginDef {
    fn schema(&self) -> &'static lumit_core::fx::EffectSchema {
        self.0
    }
}

/// Register a plug-in under `identifier`, as the OFX host does when it finds
/// one, and answer nothing — the catalogue is a process-global, so this is
/// idempotent across a test binary that runs its tests in one process.
fn a_discovered_plugin(
    identifier: &'static str,
    label: &'static str,
    params: &'static [lumit_core::fx::ParamSchema],
) {
    use lumit_core::fx::{CostClass, EffectTraits, FxCategory, MatteRole, Roi};
    let match_name: &'static str = Box::leak(format!("ofx:{identifier}").into_boxed_str());
    if lumit_core::fx::schema(match_name).is_some() {
        return;
    }
    let schema: &'static lumit_core::fx::EffectSchema =
        Box::leak(Box::new(lumit_core::fx::EffectSchema {
            match_name,
            label,
            version: 1,
            category: FxCategory::Utility,
            traits: EffectTraits {
                cost: CostClass::Heavy,
                roi: Roi::FullFrame,
                temporal: &[0],
                premultiplied: true,
                seeded: false,
                beat_input: false,
            },
            params,
            groups: &[],
            enabled_when: &[],
            matte: MatteRole::None,
        }));
    assert!(
        lumit_core::fx::BUILTIN_DEFS.register(Box::leak(Box::new(PluginDef(schema)))),
        "the plug-in registered"
    );
}

/// A plug-in control, for the two catalogue entries below.
const fn control(
    id: &'static str,
    label: &'static str,
    default: f64,
) -> lumit_core::fx::ParamSchema {
    lumit_core::fx::ParamSchema {
        id,
        label,
        kind: lumit_core::fx::ParamKind::Float {
            default,
            slider: (0.0, 100.0),
            hard: (None, None),
        },
        unit: lumit_core::fx::Unit::Raw,
    }
}

/// S_Glow's two, one of which the After Effects side hands over as a colour.
const GLOW_CONTROLS: &[lumit_core::fx::ParamSchema] = &[
    control("brightness", "Brightness", 1.0),
    control("centre_x", "Centre", 0.5),
];

/// S_DissolveLuma's one that matters: the transition itself.
const DISSOLVE_CONTROLS: &[lumit_core::fx::ParamSchema] =
    &[control("dissolve_percent", "Dissolve Percent", 0.0)];

/// One After Effects leaf of a third-party effect: a numbered match name, which
/// is why the *displayed* name is what the two builds share.
fn vendor_leaf(match_name: &str, name: &str, kind: &str, value: serde_json::Value) -> AeProp {
    AeProp {
        match_name: Some(match_name.to_string()),
        name: Some(name.to_string()),
        value_type: Some(kind.to_string()),
        value: Some(value),
        ..AeProp::default()
    }
}

/// **The user has the vendor's own OFX build installed, so the effect imports
/// as that plug-in** — the same effect rather than a likeness. The
/// controls the two builds share by displayed name come across; the one named
/// alike and shaped unlike is reported rather than coerced.
#[test]
fn a_third_party_effect_maps_direct_to_the_ofx_plugin_the_user_has_installed() {
    a_discovered_plugin(
        "com.genarts.sapphire.Lighting.S_Glow",
        "S_Glow",
        GLOW_CONTROLS,
    );

    let ran = run(&effect(
        "S_Glow",
        "S_Glow",
        vec![
            vendor_leaf("S_Glow-0004", "Brightness", "float", serde_json::json!(2.5)),
            // Named alike, shaped unlike: a colour where the plug-in declares a
            // number. Said out loud, never coerced.
            vendor_leaf(
                "S_Glow-0009",
                "Centre",
                "colour",
                serde_json::json!([1.0, 0.0, 0.0, 1.0]),
            ),
        ],
    ));

    assert!(ran.mapped, "an installed plug-in is a mapped effect");
    assert_eq!(
        ran.inst.effect.match_name, "ofx:com.genarts.sapphire.Lighting.S_Glow",
        "it is the plug-in itself, not Lumit's nearest likeness"
    );
    assert_eq!(ran.inst.effect.namespace, EffectNamespace::Ofx);
    assert_eq!(
        ran.inst
            .params
            .iter()
            .find(|p| p.id == "brightness")
            .map(|p| p.value.clone()),
        Some(EffectValue::Float(LumProperty::fixed(2.5))),
        "the control both builds name Brightness carried across"
    );
    assert!(
        ran.report.rows.iter().any(|r| matches!(
            &r.reason,
            Reason::EffectAsPlugin { match_name, carried, controls, .. }
                if match_name == "S_Glow" && *carried == 1 && *controls == 2
        )),
        "the report says which plug-in, and one of its two controls carried"
    );
    assert!(
        ran.report.rows.iter().any(|r| matches!(
            &r.reason,
            Reason::EffectParamNotCarried { param, .. } if param == "Centre"
        )),
        "the mismatched control is named rather than coerced"
    );
}

// --- the owner's dissolve curve --------------------------------------------

/// The owner's curve, hand-computed: at and above 50 % fully on, below it a
/// straight line down to nothing at 0 %.
const CURVE: &[(f64, f64)] = &[
    (0.0, 0.0),
    (10.0, 20.0),
    (25.0, 50.0),
    (49.0, 98.0),
    (50.0, 100.0),
    (75.0, 100.0),
    (100.0, 100.0),
];

/// One Sapphire dissolve, with its Dissolve Percent at `percent`.
fn a_dissolve(percent: f64) -> AeProp {
    effect(
        "S_DissolveLuma",
        "S_DissolveLuma",
        vec![vendor_leaf(
            "S_DissolveLuma-0052",
            "Dissolve Percent",
            "float",
            serde_json::json!(percent),
        )],
    )
}

/// The Completion the nearest road ended up at.
fn completion(inst: &EffectInstance) -> Option<LumProperty> {
    inst.params
        .iter()
        .find(|p| p.id == "completion")
        .and_then(|p| match &p.value {
            EffectValue::Float(v) => Some(v.clone()),
            _ => None,
        })
}

/// **The owner's dissolve curve, on both roads.**
///
/// One test rather than three, and the order inside it is the point: the OFX
/// catalogue is a process-global, so registering the dissolve plug-in closes
/// the nearest road for the rest of the binary. The two roads for one row are
/// therefore walked here in the order a machine walks them — without the
/// plug-in, then with it — instead of racing each other across threads.
#[test]
fn a_sapphire_dissolve_carries_its_amount_through_the_owners_curve_on_both_roads() {
    // --- the nearest road: every other control is still at Lumit's default,
    // and this one is the exception, because a Sapphire dissolve *is* its
    // Dissolve Percent and a transition standing in without it would sit
    // half-complete for the whole shot.
    for (input, expected) in CURVE {
        let ran = run(&a_dissolve(*input));
        assert!(ran.mapped, "the nearest effect is a mapped effect");
        assert_eq!(ran.inst.effect.match_name, "linear_wipe");
        assert_eq!(
            completion(&ran.inst),
            Some(LumProperty::fixed(*expected)),
            "Dissolve Percent {input} is Completion {expected}"
        );
        assert!(
            ran.report.rows.iter().any(|r| matches!(
                &r.reason,
                Reason::EffectParamApproximated { param, .. } if param == "Dissolve Percent"
            )),
            "the curve is named in the report rather than applied silently"
        );
    }

    // --- a keyframed dissolve converts key by key and keeps its eases: each
    // key's value goes through the curve and the shape of the move around it is
    // left alone. What that costs between two keys either side of 50 % is what
    // the report row says.
    let mut amount = keyed(
        "S_DissolveLuma-0052",
        &[(0.0, 0.0, 3.0), (1.0, 25.0, 3.0), (2.0, 60.0, 3.0)],
    );
    amount.name = Some("Dissolve Percent".to_string());
    let ran = run(&effect("S_DissolveLuma", "S_DissolveLuma", vec![amount]));
    let Some(LumProperty {
        animation: Animation::Keyframed(keys),
        ..
    }) = completion(&ran.inst)
    else {
        panic!("a keyframed Dissolve Percent is a keyframed Completion");
    };
    assert_eq!(
        keys.iter().map(|k| k.value).collect::<Vec<_>>(),
        vec![0.0, 50.0, 100.0],
        "each key's value went through the curve"
    );
    for key in &keys {
        assert!(
            matches!(key.interp_in, SideInterp::Bezier { speed, .. } if (speed - 3.0).abs() < 1e-9),
            "the ease is the one After Effects drew, untouched"
        );
    }
    assert!(
        ran.report.rows.iter().any(|r| matches!(
            &r.reason,
            Reason::EffectParamApproximated { imported_as, .. }
                if imported_as.contains("keyframe")
        )),
        "the report says the curve is met at the keys and approximated between them"
    );

    // --- the direct road: the ruling is about the dissolve, not about which
    // road it took, so the plug-in's own Dissolve Percent receives the curved
    // number too.
    a_discovered_plugin(
        "com.genarts.sapphire.Transitions.S_DissolveLuma",
        "S_DissolveLuma",
        DISSOLVE_CONTROLS,
    );
    let ran = run(&a_dissolve(25.0));
    assert_eq!(
        ran.inst.effect.match_name, "ofx:com.genarts.sapphire.Transitions.S_DissolveLuma",
        "the installed plug-in is the effect itself"
    );
    assert_eq!(
        ran.inst
            .params
            .iter()
            .find(|p| p.id == "dissolve_percent")
            .map(|p| p.value.clone()),
        Some(EffectValue::Float(LumProperty::fixed(50.0))),
        "25 % came across as the curve's 50, not as 25 — the same curve on both roads"
    );
}

// ---------------------------------------------------------------------------
// The shared machinery
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The shipped table (docs/11 §5)
// ---------------------------------------------------------------------------

/// **Every mapping the table held when it was Rust still resolves to the same
/// Lumit effect.**
///
/// The list below is the sixty-two rows as they stood on the day the table moved
/// out of `fx_colour`/`fx_distort` and into `ae-effect-map.toml`, written down
/// once so that the move could not quietly change one of them. It is the point
/// of the file's existence turned into an assertion: an editable table is only
/// worth having if editing it is the *only* way a mapping changes.
///
/// Each row is a claim about the seam end to end rather than about the file:
/// the match name goes through [`crate::map::map_effect`] exactly as an import
/// does, and the Lumit effect that comes back is compared. The two deliberate
/// placeholders are on the list too, as the effects that must NOT map.
#[test]
fn every_mapping_the_table_started_with_still_resolves() {
    // (After Effects match name, the Lumit effect it becomes — or "" for the
    // rows docs/11 §5 sends to a placeholder on purpose).
    const ROWS: &[(&str, &str)] = &[
        // The colour / blur / generate / temporal half.
        ("ADBE Gaussian Blur 2", "blur"),
        ("ADBE Motion Blur", "directional_blur"),
        ("ADBE Radial Blur", "radial_blur"),
        ("ADBE Glo2", "glow"),
        ("ADBE Easy Levels2", "levels"),
        ("ADBE HUE SATURATION", "hue_saturation"),
        ("ADBE Brightness & Contrast 2", "brightness"),
        ("ADBE Tint", "tint"),
        ("ADBE Photo Filter", "photo_filter"),
        ("ADBE Black&White", "black_and_white"),
        ("ADBE ShadowHighlight", "shadow_highlight"),
        ("ADBE Tritone", "tritone"),
        ("ADBE Posterize", "posterize"),
        ("ADBE Threshold", "threshold"),
        ("ADBE Broadcast Colors", "broadcast_safe"),
        ("ADBE Fill", "fill"),
        ("ADBE Ramp", "gradient"),
        ("ADBE Noise", "noise"),
        ("ADBE Fractal Noise", "fractal_noise"),
        ("ADBE Laser", "beam"),
        ("ADBE Lightning 2", "lightning"),
        ("APC Radio Waves", "radio_waves"),
        ("APC Vegas", "vegas"),
        ("VISINF Grain Implant", "add_grain"),
        ("ADBE Scribble Fill", "scribble"),
        ("ADBE Stroke", "stroke"),
        ("ADBE Echo", "echo"),
        ("ADBE Posterize Time", "posterize_time"),
        // The distortion / stylise / transition / utility / controls half.
        ("ADBE Geometry2", "transform"),
        ("ADBE Set Matte3", "set_matte"),
        ("ADBE Tile", "tile"),
        ("ADBE Offset", "offset"),
        ("ADBE Mirror", "mirror"),
        ("ADBE Optics Compensation", "lens_distort"),
        ("ADBE Turbulent Displace", "turbulent_displace"),
        ("ADBE Corner Pin", "corner_pin"),
        ("ADBE Displacement Map", "displacement_map"),
        ("ADBE Polar Coordinates", "polar_coordinates"),
        ("ADBE Twirl", "twirl"),
        ("ADBE Spherize", "spherize"),
        ("ADBE Ripple", "ripple"),
        ("ADBE Wave Warp", "wave_warp"),
        ("ADBE BEZMESH", "bezier_warp"),
        ("ADBE WRPMESH", "warp"),
        ("ADBE Drop Shadow", "drop_shadow"),
        ("ADBE Roughen Edges", "roughen_edges"),
        ("ADBE Median", "median"),
        ("ADBE Mosaic", "mosaic"),
        ("ADBE Find Edges", "find_edges"),
        ("ADBE Emboss", "emboss"),
        ("ADBE Texturize", "texturize"),
        ("ADBE Channel Blur", "channel_blur"),
        ("ADBE Linear Wipe", "linear_wipe"),
        ("ADBE Radial Wipe", "radial_wipe"),
        ("ADBE IRIS_WIPE", "iris_wipe"),
        ("ADBE Venetian Blinds", "venetian_blinds"),
        ("APC CardWipeCam", "card_wipe"),
        ("ADBE Slider Control", "slider_control"),
        ("ADBE Angle Control", "angle_control"),
        ("ADBE Checkbox Control", "checkbox_control"),
        ("ADBE Color Control", "colour_control"),
        ("ADBE Point Control", "point_control"),
        // Placeholders on purpose (docs/11 §5).
        ("VISINF Grain Removal", ""),
        ("ADBE Timewarp", ""),
    ];

    for (ae, lumit) in ROWS {
        let ran = run(&effect(ae, ae, Vec::new()));
        if lumit.is_empty() {
            assert!(!ran.mapped, "{ae} must stay a placeholder");
            assert_eq!(ran.inst.effect.namespace, EffectNamespace::Placeholder);
            continue;
        }
        assert!(ran.mapped, "{ae} must still map");
        assert_eq!(
            ran.inst.effect.match_name, *lumit,
            "{ae} used to become {lumit}"
        );
        assert_eq!(ran.inst.effect.namespace, EffectNamespace::Builtin);
    }
}

// ---------------------------------------------------------------------------
// The five rows that arrived with the file (docs/11 §5)
// ---------------------------------------------------------------------------

/// **Invert: Blend With Original is Mix read from the other end, and a Channel
/// Lumit has not got is reported rather than quietly obeyed.**
#[test]
fn invert_complements_its_blend_and_reports_a_channel_it_cannot_keep() {
    let rgb = run(&effect(
        "ADBE Invert",
        "Invert",
        vec![
            leaf("ADBE Invert-0001", serde_json::json!(0)),
            keyed("ADBE Invert-0002", &[(0.0, 0.0, 4.0), (2.0, 75.0, 4.0)]),
        ],
    ));
    assert!(rgb.mapped);
    assert_eq!(rgb.inst.effect.match_name, "invert");
    // 0 % blended back is the whole effect (Mix 100); 75 % back is Mix 25.
    let keys = rgb.keys("mix");
    assert!(close(keys[0].value, 100.0));
    assert!(close(keys[1].value, 25.0));
    // The handle turns over with the value — the dial runs the other way.
    assert!(matches!(
        keys[0].interp_out,
        SideInterp::Bezier { speed, .. } if close(speed, -4.0)
    ));
    assert!(
        !rgb.approximated("Channel"),
        "RGB is the channel set Lumit inverts: nothing to say"
    );

    let red = run(&effect(
        "ADBE Invert",
        "Invert",
        vec![
            leaf("ADBE Invert-0001", serde_json::json!(1)),
            leaf("ADBE Invert-0002", serde_json::json!(0)),
        ],
    ));
    assert!(
        red.mapped,
        "it still imports — the control is reported, not the effect dropped"
    );
    assert!(red.approximated("Channel"));
}

/// **Sharpen: a hundred of After Effects' units is Lumit's classic kernel, and
/// the report says the dial reads differently** (docs/11 §5's
/// undocumented-base rule).
#[test]
fn sharpen_converts_its_amount_onto_the_kernel_coefficient() {
    let ran = run(&effect(
        "ADBE Sharpen",
        "Sharpen",
        vec![keyed(
            "ADBE Sharpen-0001",
            &[(0.0, 0.0, 20.0), (2.0, 150.0, 20.0)],
        )],
    ));
    assert!(ran.mapped);
    assert_eq!(ran.inst.effect.match_name, "sharpen_simple");
    let keys = ran.keys("amount");
    assert!(close(keys[0].value, 0.0), "zero is zero on both sides");
    assert!(close(keys[1].value, 1.5));
    assert!(matches!(
        keys[0].interp_out,
        SideInterp::Bezier { speed, .. } if close(speed, 0.2)
    ));
    assert!(ran.rebased("Sharpen Amount"));
    // The neighbourhood After Effects uses, which is Lumit's default.
    assert!(close(ran.f("radius"), 1.0));
}
