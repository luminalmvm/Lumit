//! The golden bundle: the real thing, walked out of a real After Effects
//! (docs/11-AE-IMPORT.md, docs/impl/ae-import.md §5 and §6 phase 1).
//!
//! `tools/ae-bridge/fixtures/fixture.lum-bundle/` is what
//! `make-fixture.jsx` produced when the owner ran it once on After Effects
//! 26.0: two compositions, twenty-four layers, and every row of the coverage
//! checklist the builder walks step by step. `mapping.rs` proves the mapping
//! against bundles written *by hand* to describe the schema; this file proves
//! it against a bundle After Effects itself dictated, which is the only test
//! that can catch the schema being described correctly and produced
//! differently.
//!
//! Two rules run through the assertions, and both come from that being the
//! point of the file.
//!
//! **Every expected number is computed here, from the fixture's own inputs.**
//! Drop Shadow's opacity is 180 out of 255, not "70.6"; Tint's colours are the
//! captured display values pushed through the transfer function.
//! A conversion factor copied out of the mapper would make the test agree with
//! whatever the mapper does, which is not agreement at all.
//!
//! **Where a checklist row did not come through, the assertion says so.** Two
//! did not, and both are marked ROW NOT CARRIED below and owed in docs/TODO.md:
//! the roving key (After Effects did not apply it, so there is nothing in the
//! capture to import) and a 3D layer's Orientation (the capture has it and the
//! mapper has nowhere to put it). Asserting what we wish happened would hide
//! exactly the thing a golden fixture exists to reveal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use lumit_core::anim::{Animation, SideInterp};
use lumit_core::mask::{BezierPath, MaskMode};
use lumit_core::model::{
    BlendMode, Composition, Document, EffectInstance, EffectNamespace, EffectValue, Layer,
    LayerKind, LightKind, MatteChannel, ProjectItem,
};
use lumit_core::retime::Interpolation;
use lumit_core::time::Rational;
use lumit_import::{Bundle, ImportReport, Outcome, Reason, Summary};

// ---------------------------------------------------------------------------
// One open, one map, shared
// ---------------------------------------------------------------------------

/// The bundle lives in `tools/`, not in this crate: it is a megabyte of
/// capture and the tools folder is where the walker wrote it, so the test
/// reaches out to it rather than the repository carrying two copies.
fn bundle_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("ae-bridge")
        .join("fixtures")
        .join("fixture.lum-bundle")
}

/// The whole chain, run once for the file. Twenty-odd assertions against a
/// megabyte of JSON is one parse and one mapping, not twenty.
fn golden() -> &'static (Bundle, Document, ImportReport) {
    static ONCE: OnceLock<(Bundle, Document, ImportReport)> = OnceLock::new();
    ONCE.get_or_init(|| {
        let bundle = lumit_import::open_bundle(&bundle_path()).expect("the golden bundle opens");
        let (doc, report) = lumit_import::map_capture(&bundle.capture);
        (bundle, doc, report)
    })
}

fn doc() -> &'static Document {
    &golden().1
}

fn report() -> &'static ImportReport {
    &golden().2
}

fn comp<'a>(doc: &'a Document, name: &str) -> &'a Composition {
    doc.items
        .iter()
        .find_map(|item| match item {
            ProjectItem::Composition(c) if c.name == name => Some(c),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no composition named {name}"))
}

/// The outer composition, which is where all but one of the layers live.
fn fixture() -> &'static Composition {
    comp(doc(), "Fixture")
}

fn layer<'a>(comp: &'a Composition, name: &str) -> &'a Layer {
    comp.layers
        .iter()
        .find(|l| l.name == name)
        .unwrap_or_else(|| panic!("no layer named {name}"))
}

fn item_id(doc: &Document, name: &str) -> uuid::Uuid {
    doc.items
        .iter()
        .find(|i| i.name() == name)
        .unwrap_or_else(|| panic!("no item named {name}"))
        .id()
}

fn effect<'a>(layer: &'a Layer, match_name: &str) -> &'a EffectInstance {
    layer
        .effects
        .iter()
        .find(|e| e.effect.match_name == match_name)
        .unwrap_or_else(|| panic!("no effect {match_name}"))
}

fn keys(property: &lumit_core::anim::Property) -> Vec<lumit_core::anim::Keyframe> {
    match &property.animation {
        Animation::Keyframed(keys) => keys.clone(),
        other => panic!("not keyframed: {other:?}"),
    }
}

fn reported(f: impl Fn(&Reason) -> bool) -> bool {
    report().rows.iter().any(|row| f(&row.reason))
}

/// One effect parameter as the float it is, at time zero.
fn float(fx: &EffectInstance, id: &str) -> f64 {
    match fx.param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        other => panic!("{id} is not a float: {other:?}"),
    }
}

/// One channel of an effect parameter that is a colour.
fn colour(fx: &EffectInstance, id: &str) -> [f64; 4] {
    match fx.param(id) {
        Some(EffectValue::Colour(c)) => [
            c[0].value_at(0.0),
            c[1].value_at(0.0),
            c[2].value_at(0.0),
            c[3].value_at(0.0),
        ],
        other => panic!("{id} is not a colour: {other:?}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[track_caller]
fn assert_close(a: f64, b: f64) {
    assert!(close(a, b), "expected {b}, got {a}");
}

/// The sRGB transfer function, written out here rather than borrowed from the
/// mapper: a colour conversion tested against its own implementation proves
/// nothing (K-026).
fn to_linear(v: f64) -> f64 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// A closed path's perimeter, chord by chord — how AE's *count* of Vegas
/// segments becomes Lumit's segment *length* (docs/11 §5's Vegas row).
fn perimeter(path: &BezierPath) -> f64 {
    let n = path.vertices.len();
    (0..n)
        .map(|i| {
            let a = path.vertices[i].pos;
            let b = path.vertices[(i + 1) % n].pos;
            ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
        })
        .sum()
}

// ---------------------------------------------------------------------------
// The bundle itself
// ---------------------------------------------------------------------------

/// **The bundle a real After Effects wrote opens, and says which one.**
///
/// The shallow check every other test in this file assumes, and the one fact
/// no synthetic fixture can carry: `ae_version` is the build that dictated
/// this capture. The project block rides along because docs/11 §3's colour
/// flag is a fact about the project and nothing downstream can recover it —
/// and because this project is 16-bit with linear blending *off*, which is the
/// combination that raises no row (the flag is for 8-bit without it).
#[test]
fn the_golden_bundle_is_the_walkers_own_output() {
    let (bundle, doc, _) = golden();

    assert_eq!(bundle.manifest.format.as_deref(), Some("lumit-ae-bundle"));
    assert_eq!(bundle.manifest.version.as_deref(), Some("1.0.0"));
    assert_eq!(bundle.manifest.ae_version.as_deref(), Some("26.0x67"));
    assert_eq!(bundle.manifest.bridge_version.as_deref(), Some("1.0.0"));

    let project = bundle.capture.project.as_ref().expect("a project block");
    assert_eq!(project.bits_per_channel, Some(16));
    assert_eq!(project.linear_blending, Some(false));
    assert_eq!(project.expression_engine.as_deref(), Some("javascript-1.0"));
    assert!(
        !reported(|r| matches!(r, Reason::ProjectBlendingDiffers { .. })),
        "sixteen bits is not the eight-bit blending difference"
    );

    // Two comps, twenty-two items, twenty-four layers between them.
    assert_eq!(bundle.capture.comps.len(), 2);
    assert_eq!(doc.items.len(), 22);
    assert_eq!(
        fixture().layers.len() + comp(doc, "Fixture inner").layers.len(),
        24
    );
}

// ---------------------------------------------------------------------------
// §5: nested comps, the item tree, the comp settings
// ---------------------------------------------------------------------------

/// **§5 row: nested comps (A contains B), and the folder tree around them.**
///
/// `make-fixture.jsx` puts "Fixture inner" inside "Fixture folder" and adds it
/// to "Fixture" twice. The join being proved is the schema's own: a comp
/// carries no name in `comps[]`, so it is only ever reached through an id
/// shared with `items[]` — and a precomp layer names it through that same id.
#[test]
fn the_nested_comp_its_folder_and_its_two_precomp_layers_come_across() {
    let doc = doc();

    let inner = comp(doc, "Fixture inner");
    assert_eq!((inner.width, inner.height), (320, 240));
    assert_eq!(inner.duration.0, Rational::new(4, 1).unwrap());
    assert_eq!((inner.frame_rate.num(), inner.frame_rate.den()), (25, 1));

    let folder = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Folder(f) if f.name == "Fixture folder" => Some(f),
            _ => None,
        })
        .expect("the fixture folder");
    assert_eq!(folder.children, vec![inner.id]);

    // Eighteen solids in the Solids folder AE made for them.
    let solids = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Folder(f) if f.name == "Solids" => Some(f),
            _ => None,
        })
        .expect("AE's own Solids folder");
    assert_eq!(solids.children.len(), 18);

    // Both precomp layers resolve to the same nested composition.
    for name in ["retimed precomp", "stretched precomp"] {
        assert_eq!(
            layer(fixture(), name).kind,
            LayerKind::Precomp { comp: inner.id },
            "{name} is a Precomp layer pointing at the nested comp"
        );
    }

    // The inner comp's own animation arrived: 100 → 25 → 100 opacity.
    let opacity = keys(&layer(inner, "inner base").transform.opacity);
    assert_eq!(opacity.len(), 3);
    assert_eq!(
        opacity.iter().map(|k| k.value).collect::<Vec<_>>(),
        [100.0, 25.0, 100.0]
    );
    assert_eq!(opacity[1].time, Rational::new(2, 1).unwrap());
}

/// **The composition's own settings, its shutter, and its background.**
///
/// Every number here is one `make-fixture.jsx` wrote by hand, so a mistake is
/// a plain transcription slip — the kind a test catches and a reading does
/// not. The background is the one that converts: After Effects reports it in
/// the project's display space and Lumit stores scene-linear light (K-026).
#[test]
fn the_composition_keeps_its_settings_shutter_and_background() {
    let c = fixture();

    assert_eq!((c.width, c.height), (640, 360));
    assert_eq!((c.frame_rate.num(), c.frame_rate.den()), (25, 1));
    assert_eq!(c.duration.0, Rational::new(10, 1).unwrap());

    assert!(c.motion_blur.enabled);
    assert_eq!(c.motion_blur.shutter_angle, 200.0);
    assert_eq!(c.motion_blur.shutter_phase, -100.0);
    assert_eq!(c.motion_blur.samples, 24);
    assert_eq!(
        c.extra.get("ae").and_then(|ae| ae.get("adaptive_limit")),
        Some(&serde_json::json!(64)),
        "AE's adaptive sample limit has no Lumit field and is kept"
    );

    // bgColor [0.05, 0.05, 0.06] — AE rounds each to the nearest 8-bit step
    // before reporting, so the expected value is that step, linearised.
    let step = |eighth_bit: f64| to_linear(eighth_bit / 255.0);
    assert_close(f64::from(c.background.0[0]), step(13.0));
    assert_close(f64::from(c.background.0[2]), step(15.0));

    // "Preserve frame rate when nested" has no Lumit switch and is a row.
    assert!(reported(|r| matches!(
        r,
        Reason::NestedPreserveIgnored {
            fps: true,
            resolution: false
        }
    )));
}

/// **§5 row: solids, and their colours converted to scene-linear light.**
///
/// The conversion runs on every solid in the project, so one wrong transfer
/// function is a project-wide colour shift. Three of `make-fixture.jsx`'s own
/// colours are checked against the sRGB curve computed here.
#[test]
fn every_solids_colour_arrives_in_scene_linear_light() {
    let doc = doc();
    let solid = |name: &str| match doc.item(item_id(doc, name)) {
        Some(ProjectItem::Solid(s)) => s.clone(),
        other => panic!("{name} is not a solid: {other:?}"),
    };

    // addSolid([0.5, 0.5, 0.5], "bg", 640, 360) — a mid grey.
    let bg = solid("bg");
    assert_eq!((bg.width, bg.height), (640, 360));
    for channel in 0..3 {
        assert_close(f64::from(bg.colour.0[channel]), to_linear(0.5));
    }
    assert_eq!(bg.colour.0[3], 1.0);

    // [0.9, 0.3, 0.3], as AE's float32 rounding leaves them.
    let multiply = solid("blend multiply");
    assert_close(f64::from(multiply.colour.0[0]), to_linear(0.899_999_976));
    assert_close(f64::from(multiply.colour.0[1]), to_linear(0.300_000_012));
    assert_eq!((multiply.width, multiply.height), (320, 200));

    // The item keeps After Effects' own id, so a re-import can name it.
    assert_eq!(
        multiply.extra.get("ae").and_then(|ae| ae.get("id")),
        Some(&serde_json::json!(31))
    );
}

// ---------------------------------------------------------------------------
// §5: parenting, and the two layer kinds AE backs with a solid
// ---------------------------------------------------------------------------

/// **§5 rows: the parenting chain, the null, and the adjustment layer.**
///
/// The chain is child B → child A → rig null, by *index* in the capture and by
/// id here, and the indices point up the stack at layers that had not been
/// built when the child was — which is why the ids are handed out first.
///
/// The null and the adjustment layer ride along because the golden bundle is
/// what revealed them: After Effects backs both with a solid *item* of its
/// own, so a mapper that lets the source item decide the layer kind imports a
/// rig's null as the white card it is made of and an adjustment layer as an
/// opaque solid over the whole comp. Both are invisible in After Effects and
/// were very visible here; the layer's own kind now wins for these two.
#[test]
fn the_parenting_chain_the_null_and_the_adjustment_layer() {
    let c = fixture();

    let null = layer(c, "rig null");
    assert_eq!(null.kind, LayerKind::Null, "not its backing solid");
    let child_a = layer(c, "child A");
    let child_b = layer(c, "child B");
    assert_eq!(child_a.parent, Some(null.id));
    assert_eq!(child_b.parent, Some(child_a.id));
    assert_eq!(null.parent, None);

    let adjustment = layer(c, "adjustment");
    assert_eq!(adjustment.kind, LayerKind::Adjustment);
    // And its Gaussian blur came with it, its pixels carried as px@comp.
    let blur = effect(adjustment, "blur");
    assert_close(float(blur, "radius"), 6.0);
}

// ---------------------------------------------------------------------------
// §5: the keyframe variety
// ---------------------------------------------------------------------------

/// **§5 row: position keys with bezier ease, a hold key, and spatial
/// tangents.**
///
/// `make-fixture.jsx` eases key 1 with (speed 0, influence 20) in and
/// (0, 75) out, mirrors that on key 5, and makes key 4's out-side a hold.
/// After Effects' influence is a percentage in 0.1–100 and Lumit's is a
/// fraction in (0, 1], so 20 must arrive as 0.2 and not as 20 — a curve a
/// hundred times too eager is the classic import failure.
///
/// **ROW NOT CARRIED — roving.** The builder's last line for this property is
/// `setRovingAtKey(2, true)`, and the capture records `roving: false` on every
/// key: After Effects did not apply it, so there is nothing to import. The
/// walker reads `keyRoving` correctly, which is why the assertion below is on
/// the *capture*. Owed in docs/TODO.md against one more AE sitting.
#[test]
fn the_position_keys_keep_their_ease_their_hold_and_report_the_motion_path() {
    let child_a = layer(fixture(), "child A");
    let x = keys(&child_a.transform.position_x);
    let y = keys(&child_a.transform.position_y);

    assert_eq!(x.len(), 5);
    assert_eq!(
        x.iter().map(|k| k.value).collect::<Vec<_>>(),
        [100.0, 200.0, 320.0, 440.0, 560.0]
    );
    assert_eq!(
        y.iter().map(|k| k.value).collect::<Vec<_>>(),
        [100.0, 160.0, 120.0, 240.0, 180.0]
    );
    for (i, key) in x.iter().enumerate() {
        assert_eq!(key.time, Rational::new(i as i64, 1).unwrap());
    }

    // AE's 20% and 75% are Lumit's 0.2 and 0.75, on both axes of the same key.
    for lane in [&x, &y] {
        assert_eq!(
            lane[0].interp_in,
            SideInterp::Bezier {
                speed: 0.0,
                influence: 20.0 / 100.0
            }
        );
        assert_eq!(
            lane[0].interp_out,
            SideInterp::Bezier {
                speed: 0.0,
                influence: 75.0 / 100.0
            }
        );
        // Mirrored on the last key.
        assert_eq!(
            lane[4].interp_in,
            SideInterp::Bezier {
                speed: 0.0,
                influence: 75.0 / 100.0
            }
        );
        assert_eq!(
            lane[4].interp_out,
            SideInterp::Bezier {
                speed: 0.0,
                influence: 20.0 / 100.0
            }
        );
        // The hold key, and it evaluates as one: flat until the next key.
        assert_eq!(lane[3].interp_out, SideInterp::Hold);
    }
    assert_eq!(child_a.transform.position_x.value_at(3.5), 440.0);
    assert_eq!(child_a.transform.position_y.value_at(3.9), 240.0);

    // AE clamps the in-side influence of a key it has just made a hold to
    // zero, which is outside its own 0.1–100 range; Lumit's floor catches it
    // rather than dividing by nothing.
    assert_eq!(
        x[3].interp_in,
        SideInterp::Bezier {
            speed: 0.0,
            influence: 1e-3
        }
    );

    // Lumit animates each axis on its own, so AE's motion path has nowhere to
    // go — a report row, not a silent loss.
    assert!(reported(|r| matches!(r, Reason::SpatialTangentsFlattened)));

    // ROW NOT CARRIED: roving never made it into the capture at all.
    let capture_keys = capture_position_keys("child A");
    assert!(
        capture_keys.iter().all(|k| k.roving == Some(false)),
        "After Effects did not apply setRovingAtKey — see docs/TODO.md"
    );
}

/// The capture's own Position keyframes for one layer of the outer comp, for
/// the two assertions that are about what After Effects *said* rather than
/// what the mapping made of it.
fn capture_position_keys(layer_name: &str) -> Vec<lumit_import::capture::Keyframe> {
    let capture = &golden().0.capture;
    let comp = capture
        .comps
        .iter()
        .find(|c| c.id == Some(17))
        .expect("the outer comp");
    let layer = comp
        .layers
        .iter()
        .find(|l| l.name.as_deref() == Some(layer_name))
        .expect("the layer");
    let transform = layer
        .properties
        .iter()
        .find(|p| p.match_name.as_deref() == Some("ADBE Transform Group"))
        .expect("a transform group");
    transform
        .children()
        .iter()
        .find(|p| p.match_name.as_deref() == Some("ADBE Position"))
        .and_then(|p| p.keyframes.clone())
        .unwrap_or_default()
}

/// **§5 row: separated position, whose animation is on the followers.**
///
/// After Effects' own trap, and the one the golden bundle confirms the shape
/// of: the leader still reports a still value `[80, 60, 0]`, so reading it is
/// not an error anybody sees — it is a moving layer that quietly stopped
/// moving. X and Y also have *different* key counts and times here, which is
/// the whole point of separating them.
#[test]
fn a_separated_position_animates_on_its_own_followers() {
    let child_b = layer(fixture(), "child B");

    let x = keys(&child_b.transform.position_x);
    assert_eq!((x[0].value, x[1].value), (80.0, 520.0));
    assert_eq!(x[1].time, Rational::new(3, 1).unwrap());

    let y = keys(&child_b.transform.position_y);
    assert_eq!((y[0].value, y[1].value), (60.0, 300.0));
    assert_eq!(y[1].time, Rational::new(2, 1).unwrap(), "its own last key");

    // Z was never separated away from its still zero.
    assert_eq!(
        child_b.transform.position_z.animation,
        Animation::Static(0.0)
    );
}

/// **§5 row: rotation and opacity keys.**
///
/// The plain case, on the layer `make-fixture.jsx` gives both to. Worth its
/// own assertion because the two lanes come out of different AE match names
/// (`ADBE Rotate Z` and `ADBE Opacity`) and land on differently named Lumit
/// fields, which is exactly where a transcription slip lives.
#[test]
fn rotation_and_opacity_keys_come_across() {
    let multiply = layer(fixture(), "blend multiply");

    let rotation = keys(&multiply.transform.rotation);
    assert_eq!(rotation.len(), 2);
    assert_eq!((rotation[0].value, rotation[1].value), (0.0, 180.0));
    assert_eq!(rotation[1].time, Rational::new(2, 1).unwrap());
    assert_eq!(multiply.transform.rotation.value_at(1.0), 90.0);

    let opacity = keys(&multiply.transform.opacity);
    assert_eq!((opacity[0].value, opacity[1].value), (100.0, 40.0));
    assert_eq!(multiply.transform.opacity.value_at(1.0), 70.0);
}

/// **§5 row: one enabled expression and one disabled one.**
///
/// Exactly After Effects' own behaviour, and the reason both states have to
/// survive. `child A`'s Opacity carries `50 + 25` switched on, so it drives
/// the property; `child B`'s Rotation carries `time * 45` switched off, so the
/// text is kept and drives nothing.
#[test]
fn an_expression_drives_the_property_only_when_it_was_switched_on() {
    let c = fixture();

    let opacity = &layer(c, "child A").transform.opacity;
    assert_eq!(
        opacity.animation,
        Animation::Expression("50 + 25".to_string())
    );
    assert!(reported(|r| matches!(r, Reason::ExpressionCarried)));

    let rotation = &layer(c, "child B").transform.rotation;
    assert_eq!(rotation.animation, Animation::Static(0.0));
    assert_eq!(
        rotation.extra.get("ae").and_then(|ae| ae.get("expression")),
        Some(&serde_json::json!("time * 45")),
        "kept so it can be switched back on"
    );
    assert!(reported(|r| matches!(r, Reason::ExpressionDisabledCarried)));
}

// ---------------------------------------------------------------------------
// §5: the two stretches, the time remap, the frame blending
// ---------------------------------------------------------------------------

/// **§5 rows: stretch at 50% and at −100%, as Retimes with the right ends.**
///
/// A layer at 50% plays its source twice as fast, which is a straight line of
/// slope two from layer time to source time — the four-second nested comp
/// squeezed into two.
///
/// −100% is the case only a real After Effects could have shown us. AE does
/// the turning round *itself*: setting the stretch reflects the layer about
/// its own zero, which arrives here as a bar sitting entirely before comp time
/// zero with its two ends the other way round. Reading the ends in order and
/// then applying AE's plain arithmetic — source time is layer time times the
/// rate — walks the source from its last moment back to its first. The
/// reflection the mapper used to do on top of that was written against a
/// bundle nobody had seen, and doubled the turn.
#[test]
fn both_stretches_become_retimes_with_the_right_endpoints() {
    let c = fixture();

    // 50%: two seconds of layer, four seconds of source.
    let stretched = layer(c, "stretched precomp");
    let retime = stretched
        .retime
        .as_ref()
        .expect("a Retime from the stretch");
    assert_eq!(stretched.in_point.0, Rational::ZERO);
    assert_eq!(stretched.out_point.0, Rational::new(2, 1).unwrap());
    assert_eq!(retime.value_at(0.0), 0.0);
    assert_eq!(retime.value_at(1.0), 2.0);
    assert_eq!(retime.value_at(2.0), 4.0);
    assert!(reported(|r| matches!(
        r,
        Reason::StretchAsRetime { percent } if close(*percent, 50.0)
    )));

    // −100%: the bar sits before zero, read the right way round.
    let reversed = layer(c, "reversed");
    assert!(
        reversed.in_point < reversed.out_point,
        "the ends are in order"
    );
    assert!(
        close(reversed.in_point.0.to_f64(), -10.000_32)
            && close(reversed.out_point.0.to_f64(), -0.000_32),
        "AE reflected the ten-second solid about its own zero: {:?}..{:?}",
        reversed.in_point,
        reversed.out_point
    );
    assert!(
        !reported(|r| matches!(r, Reason::LayerSpanRepaired)),
        "a reversed bar is not a damaged one"
    );

    // Source time is minus layer time, all the way along.
    let retime = reversed.retime.as_ref().expect("a Retime from the stretch");
    for t in [-10.0, -5.0, -1.0] {
        assert_close(retime.value_at(t), -t);
    }
    assert!(reported(|r| matches!(
        r,
        Reason::StretchAsRetime { percent } if close(*percent, -100.0)
    )));
}

/// **§5 rows: time remap with a hold key, and both frame-blending modes.**
///
/// AE's time-remap value graph and Lumit's Retime value graph are the same
/// mathematical object, so this is a value copy and the hold key *is* a freeze
/// without anything having to translate it. Switching remapping on makes AE
/// add its own two keys at the ends, so the curve is 0 → 1 (held) → 4: the
/// held span returns the same source moment for ever, which is a frozen frame
/// by definition.
///
/// Frame blending is a separate control and lands beside the map, never inside
/// it (docs/04 §10): Frame Mix is the crossfade, Pixel Motion is flow, and a
/// layer with neither is nearest.
#[test]
fn the_time_remaps_hold_is_a_freeze_and_frame_blending_lands_beside_it() {
    let c = fixture();
    let retimed = layer(c, "retimed precomp");
    let map = retimed.retime.as_ref().expect("a Retime");

    let keys = keys(map);
    assert_eq!(keys.len(), 3, "AE's own end keys plus the one we set");
    assert_eq!((keys[0].time, keys[0].value), (Rational::ZERO, 0.0));
    assert_eq!(keys[1].time, Rational::new(2, 1).unwrap());
    assert_eq!(keys[1].value, 1.0);
    assert_eq!(keys[1].interp_in, SideInterp::Hold);
    assert_eq!(keys[1].interp_out, SideInterp::Hold);
    assert_eq!(keys[2].time, Rational::new(4, 1).unwrap());

    // Across the whole held span, the same source moment.
    assert_eq!(map.value_at(2.5), 1.0);
    assert_eq!(map.value_at(3.9), 1.0);

    assert_eq!(retimed.interpolation, Interpolation::Blend, "Frame Mix");
    assert!(matches!(
        layer(c, "stretched precomp").interpolation,
        Interpolation::Flow(_)
    ));
    assert!(reported(|r| matches!(r, Reason::FlowEngineDiffers)));
    assert_eq!(layer(c, "bg").interpolation, Interpolation::Nearest);
}

// ---------------------------------------------------------------------------
// §5: mattes, masks, markers
// ---------------------------------------------------------------------------

/// **§5 rows: both generations of matte, normalised.**
///
/// One thing only a live After Effects could tell us: 26.0 records the modern
/// form for *both*, so the legacy `trackMatteType`-only assignment the builder
/// makes for the luma matte comes back naming its layer outright. Both
/// therefore arrive here as one thing — a chosen layer, a channel, and an
/// inversion — which is the normalisation docs/11 §3 asks for. (The genuinely
/// legacy above-layer form, which older projects still carry, keeps its own
/// test in `mapping.rs` against `edges.lum-bundle`.)
///
/// The alpha matte is the interesting one: its source is layer 3 and the
/// matted layer is layer 16, so a mapper that assumed "the layer above" would
/// pick up the wrong row entirely.
#[test]
fn both_matte_generations_normalise_to_a_layer_a_channel_and_an_inversion() {
    let c = fixture();

    let alpha = layer(c, "alpha matted").matte.as_ref().expect("a matte");
    assert_eq!(alpha.layer, layer(c, "alpha matte source").id);
    assert_eq!(alpha.channel, MatteChannel::Alpha);
    assert!(alpha.inverted, "ALPHA_INVERTED is Alpha plus inverted");

    let luma = layer(c, "luma matted").matte.as_ref().expect("a matte");
    assert_eq!(luma.layer, layer(c, "luma matte source").id);
    assert_eq!(luma.channel, MatteChannel::Luma);
    assert!(!luma.inverted);

    // After Effects switches a matte layer's own video off; Lumit keeps that
    // exactly as it stood (docs/11 §3) rather than forcing it back on.
    assert!(!layer(c, "alpha matte source").switches.visible);
    assert!(!layer(c, "luma matte source").switches.visible);
    assert!(!reported(|r| matches!(
        r,
        Reason::MatteTargetMissing { .. }
    )));
}

/// **§5 row: two masks — modes, feather, inversion, and an animated path.**
///
/// The path is the only property value with structure rather than numbers, so
/// it is the one worth proving vertex by vertex: After Effects hands over
/// three parallel arrays and Lumit stores one vertex carrying both its
/// tangents. The second mask has no still value at all — its shape is
/// keyframed — so the drawn path has to come from its first key.
#[test]
fn the_two_masks_keep_their_modes_feather_inversion_and_animated_path() {
    let host = layer(fixture(), "fx host");
    assert_eq!(host.masks.len(), 2);

    let add = &host.masks[0];
    assert_eq!(add.name, "mask add");
    assert_eq!(add.mode, MaskMode::Add);
    assert!(!add.inverted);
    assert_eq!(add.opacity.value_at(0.0), 80.0);
    assert_eq!(add.expansion.value_at(0.0), 4.0);
    assert_eq!(add.feather.value_at(0.0), 0.0);
    assert_eq!(add.path.vertices.len(), 4);
    assert!(add.path.closed);
    assert_eq!(add.path.vertices[0].pos, (40.0, 40.0));
    assert_eq!(add.path.vertices[1].pos, (280.0, 40.0));
    assert_eq!(add.path.vertices[0].tan_out, (0.0, 0.0));

    let subtract = &host.masks[1];
    assert_eq!(subtract.name, "mask subtract");
    assert_eq!(subtract.mode, MaskMode::Subtract);
    assert!(subtract.inverted);
    // 12 × 12: one width, and no "the axes differ" row.
    assert_eq!(subtract.feather.value_at(0.0), 12.0);
    assert!(!reported(|r| matches!(
        r,
        Reason::MaskFeatherAxesDiffer { .. }
    )));

    assert_eq!(subtract.path_keys.len(), 2);
    assert_eq!(subtract.path_keys[0].time, Rational::ZERO);
    assert_eq!(subtract.path_keys[1].time, Rational::new(2, 1).unwrap());
    assert_eq!(subtract.path_keys[0].path.vertices[0].pos, (320.0, 80.0));
    assert_eq!(subtract.path_keys[1].path.vertices[1].pos, (560.0, 60.0));
    assert_eq!(
        subtract.path.vertices[0].pos,
        (320.0, 80.0),
        "with no still value, the drawn shape is the first key's"
    );
}

/// **§5 row: comp and layer markers, with their comments and durations.**
///
/// A marker's *duration* is the half an importer forgets, because a marker
/// without one still looks right. AE reports a durationless marker as zero
/// seconds, which is not the same thing as a span and must not import as one.
#[test]
fn comp_and_layer_markers_carry_their_comment_and_their_duration() {
    let c = fixture();

    assert_eq!(c.markers.len(), 1);
    assert_eq!(c.markers[0].label, "comp marker");
    assert_eq!(c.markers[0].time.0, Rational::new(2, 1).unwrap());
    assert_eq!(c.markers[0].duration, Some(Rational::new(5, 4).unwrap()));

    let host = layer(c, "fx host");
    assert_eq!(host.markers.len(), 2);
    assert_eq!(host.markers[0].label, "fx marker");
    assert_eq!(host.markers[0].time.0, Rational::new(1, 1).unwrap());
    assert_eq!(host.markers[0].duration, Some(Rational::new(1, 2).unwrap()));
    assert_eq!(host.markers[1].label, "second marker");
    assert_eq!(host.markers[1].duration, None, "zero is not a span");

    let inner = comp(doc(), "Fixture inner");
    assert_eq!(inner.markers[0].label, "inner marker");
}

// ---------------------------------------------------------------------------
// §5: blend modes, switches, the 3D trio, text
// ---------------------------------------------------------------------------

/// **§5 row: the blend-mode spread, including Dissolve's fallback.**
///
/// Three of the four cross one for one. Dissolve is the row docs/11 §4 puts at
/// a documented fallback: Lumit has no stochastic transparency, so it imports
/// as Normal and says so. The picture changes either way — the only
/// unacceptable outcome is a quiet one.
#[test]
fn the_blend_mode_spread_arrives_and_dissolve_falls_back_with_a_row() {
    let c = fixture();

    assert_eq!(layer(c, "blend multiply").blend, BlendMode::Multiply);
    assert_eq!(layer(c, "blend screen").blend, BlendMode::Screen);
    assert_eq!(layer(c, "blend overlay").blend, BlendMode::Overlay);
    assert_eq!(layer(c, "bg").blend, BlendMode::Normal);

    assert_eq!(layer(c, "blend dissolve").blend, BlendMode::Normal);
    assert!(reported(|r| matches!(
        r,
        Reason::BlendModeUnavailable { ae_mode } if ae_mode == "DISSOLVE"
    )));
}

/// **§5 row: the switch states, and the two with no counterpart.**
///
/// Shy, solo, lock, motion blur and the guide flag (K-497) cross straight
/// over. Draft quality and "preserve underlying transparency" have no Lumit
/// switch, and each changes what a comp looks like, so each is a row rather
/// than a silent drop.
#[test]
fn the_switches_cross_over_and_the_ones_with_no_counterpart_are_reported() {
    let c = fixture();

    assert!(layer(c, "blend overlay").switches.shy);
    assert!(layer(c, "child B").switches.solo);
    assert!(layer(c, "bg").switches.locked);
    assert!(layer(c, "blend screen").switches.motion_blur);
    assert!(layer(c, "3d card").switches.three_d);
    assert!(layer(c, "retimed precomp").switches.collapse);
    assert!(layer(c, "fx host").switches.fx);

    assert!(
        layer(c, "guide").switches.guide,
        "AE's guide flag is Lumit's guide switch"
    );
    assert!(
        layer(c, "guide").switches.visible,
        "a guide layer still draws in the Viewer"
    );
    assert!(reported(|r| matches!(
        r,
        Reason::LayerQualityIgnored { quality } if quality == "DRAFT"
    )));
    assert!(reported(|r| matches!(
        r,
        Reason::PreserveTransparencyNotSupported
    )));
}

/// **§5 row: the 3D layer, the two-node camera and the light — as far as they
/// map.**
///
/// What does come across: the 3D switch and the layer's Z, the layer's
/// **Orientation** on the rotation lanes (K-625 — the rotations here are
/// zero, so the orientation is exactly what they describe), the camera's Zoom
/// out of its options group, and the light's kind, colour, intensity and cone.
/// Two of those convert — AE's intensity is a percentage where 100 is unity,
/// and its cone angle is the *full* angle where Lumit's is the half.
///
/// **ROWS NOT CARRIED**, all owed in docs/TODO.md and all revealed here:
///
/// - Material Options' **Casts Shadows** has no Lumit field and, unlike
///   everything else with none, raises no report row;
/// - the camera's **Point of Interest** lands on the anchor-point lanes,
///   because After Effects stores it under `ADBE Anchor Point` — its two-node
///   flag survives in the `ae` namespace, and now says so in the report;
/// - the camera's **Depth of Field, Aperture and Focus Distance** are dropped
///   without a row.
#[test]
fn the_3d_layer_the_camera_and_the_light_come_across_as_far_as_they_map() {
    let c = fixture();

    let card = layer(c, "3d card");
    assert!(card.switches.three_d);
    assert_eq!(card.transform.position_z.value_at(0.0), -150.0);
    // Orientation [0, 30, 0] onto the rotation lanes, the layer's own
    // rotations being zero (K-625).
    assert_eq!(card.transform.rotation_x.value_at(0.0), 0.0);
    assert_eq!(card.transform.rotation_y.value_at(0.0), 30.0);
    assert_eq!(card.transform.rotation.value_at(0.0), 0.0);

    let camera = layer(c, "camera");
    let LayerKind::Camera { zoom, .. } = &camera.kind else {
        panic!("a camera");
    };
    assert_eq!(zoom.value_at(0.0), 800.0, "read out of Camera Options");
    assert_eq!(
        camera.extra.get("ae").and_then(|ae| ae.get("auto_orient")),
        Some(&serde_json::json!("CAMERA_OR_POINT_OF_INTEREST")),
        "the two-node flag rides in the ae namespace"
    );
    assert!(
        reported(|r| matches!(r, Reason::PointOfInterestNotCarried)),
        "and the aim it stands for is reported as not carried"
    );
    // ROW NOT CARRIED: the Point of Interest arrives as the anchor point.
    assert_eq!(camera.transform.anchor_x.value_at(0.0), 320.0);
    assert_eq!(camera.transform.anchor_y.value_at(0.0), 180.0);

    let LayerKind::Light { light } = &layer(c, "key light").kind else {
        panic!("a light");
    };
    assert_eq!(light.kind, LightKind::Spot);
    assert_close(light.intensity.value_at(0.0), 75.0 / 100.0);
    assert_close(light.cone_deg.value_at(0.0), 60.0 / 2.0);
    // [1, 0.9, 0.8] in the project's display space.
    assert_close(light.colour[0].value_at(0.0), 1.0);
    assert_close(light.colour[1].value_at(0.0), to_linear(0.899_999_976));
    assert_close(light.colour[2].value_at(0.0), to_linear(0.800_000_012));
}

/// **§5 rows: the text layer's source string, and the shape layer's slot.**
///
/// Lumit's text layer has the words, the size and the fill colour, so those
/// three convert (the fill through the same sRGB curve as everything else) and
/// the rest of the styling — the stroke, the tracking, the justification the
/// builder sets — is a report row. The shape layer keeps its place, its
/// transform and its parenting and draws nothing, which is also a row.
#[test]
fn the_text_layers_words_arrive_and_the_shape_layer_keeps_its_slot() {
    let c = fixture();

    let LayerKind::Text { document } = &layer(c, "Lumit fixture").kind else {
        panic!("a text layer");
    };
    assert_eq!(document.text, "Lumit fixture");
    assert_eq!(document.size, 48.0);
    // fillColor [1, 0.55, 0.1].
    assert_close(f64::from(document.fill.0[1]), to_linear(0.550_000_011));
    assert_close(f64::from(document.fill.0[2]), to_linear(0.100_000_001));
    assert!(reported(|r| matches!(r, Reason::TextStylingNotMapped)));

    assert_eq!(
        layer(c, "shape").kind,
        LayerKind::Shape {
            contents: Vec::new()
        }
    );
    assert!(reported(|r| matches!(r, Reason::ShapeContentsNotMapped)));
}

// ---------------------------------------------------------------------------
// §5: the effect spread, parameter by converted parameter
// ---------------------------------------------------------------------------

/// **The blur, the colour effects and the generators, with their numbers
/// converted.**
///
/// Every figure below is worked out here from what `make-fixture.jsx` set and
/// what the composition is, never copied from the mapper: a blur radius is 40
/// After Effects pixels and stays 40 (px@comp, K-419), Fill's opacity is a
/// bare 0–1 factor where Lumit reads a per cent, and Tint's two colours cross
/// into scene-linear light.
#[test]
fn the_colour_and_generate_effects_convert_every_parameter() {
    let host = layer(fixture(), "fx host");

    // Gaussian Blur: keyframed, pixels on both sides, and no rebase row.
    let blur = effect(host, "blur");
    let radius = match blur.param("radius") {
        Some(EffectValue::Float(p)) => p,
        other => panic!("radius is a float: {other:?}"),
    };
    let radius_keys = keys(radius);
    assert_eq!(radius_keys.len(), 2);
    assert_eq!(radius_keys[0].value, 0.0);
    assert_close(radius_keys[1].value, 40.0);
    assert_eq!(radius_keys[1].time, Rational::new(2, 1).unwrap());
    assert!(!reported(|r| matches!(
        r,
        Reason::EffectParamRebased { effect, param }
            if effect == "Gaussian Blur" && param == "Blurriness"
    )));

    // Tint, and the one instance switched off in After Effects.
    let tint = effect(host, "tint");
    assert!(!tint.enabled, "a switched-off effect imports switched off");
    assert_close(colour(tint, "black")[2], to_linear(0.200_000_003));
    assert_close(colour(tint, "white")[1], to_linear(0.899_999_976));
    assert_close(colour(tint, "white")[2], to_linear(0.5));
    assert_close(float(tint, "mix"), 60.0);

    // Fill: AE's Opacity is a 0–1 factor, Lumit's Mix is a per cent.
    let fill = effect(host, "fill");
    assert_close(float(fill, "mix"), 0.800_000_012 * 100.0);
    assert_close(colour(fill, "colour")[1], 1.0);
    assert_close(colour(fill, "colour")[2], to_linear(0.25));

    // Fractal Noise: two option lists and the one rebased scale.
    let noise = effect(host, "fractal_noise");
    assert_eq!(
        noise.param("noise_type"),
        Some(&EffectValue::Choice(0)),
        "AE's Block is Lumit's first entry"
    );
    assert_eq!(
        noise.param("fractal_type"),
        Some(&EffectValue::Choice(1)),
        "AE's Turbulent Smooth folds onto Lumit's Turbulent"
    );
    assert_close(float(noise, "contrast"), 140.0);
    assert_close(float(noise, "complexity"), 3.0);
    assert_close(float(noise, "evolution"), 90.0);
    // AE's 100 is Lumit's 200 on the anchor docs/11 §5 records.
    assert_close(float(noise, "scale"), 100.0 * 2.0);
    assert!(reported(|r| matches!(
        r,
        Reason::EffectParamApproximated { effect, param, .. }
            if effect == "Fractal Noise" && param == "Fractal Type"
    )));

    // Levels and Hue/Saturation: the master group, values unchanged.
    let levels = effect(host, "levels");
    assert_close(float(levels, "master_in_black"), 0.050_000_000_745);
    assert_close(float(levels, "master_gamma"), 1.200_000_047_684);
    assert_close(float(levels, "master_in_white"), 1.0);

    let hue = effect(host, "hue_saturation");
    assert_close(float(hue, "master_hue"), 30.0);
    assert_close(float(hue, "master_saturation"), 15.0);
    assert_close(float(hue, "reds_hue"), 0.0);
}

/// **The Transform, the Drop Shadow, the Vegas and the Scribble.**
///
/// The four whose conversions are arithmetic rather than a copy: AE's Drop
/// Shadow opacity is 0–255 where Lumit's is a per cent; Vegas counts segments
/// round a contour where Lumit spaces them along it, so the count becomes a
/// length through the mask's own perimeter; Scribble and Vegas both name a
/// mask by index, which has to resolve to the same id the mask itself was
/// given (K-408).
#[test]
fn the_transform_shadow_and_the_two_mask_reading_effects_convert() {
    let host = layer(fixture(), "fx host");

    let transform = effect(host, "transform");
    assert_close(float(transform, "anchor_x"), 320.0);
    assert_close(float(transform, "anchor_y"), 180.0);
    assert_close(float(transform, "position_x"), 200.0);
    assert_close(float(transform, "position_y"), 150.0);
    assert_close(float(transform, "rotation"), 15.0);
    assert_close(float(transform, "opacity"), 90.0);
    // Uniform Scale was on, so both axes take AE's single figure.
    assert_close(float(transform, "scale_x"), 100.0);
    assert_close(float(transform, "scale_y"), 100.0);

    let shadow = effect(host, "drop_shadow");
    assert_close(float(shadow, "opacity"), 180.0 * 100.0 / 255.0);
    assert_close(float(shadow, "direction"), 200.0);
    assert_close(float(shadow, "distance"), 12.0);
    assert_close(float(shadow, "softness"), 8.0);

    // Both effects name mask 1, which is the layer's first mask.
    let first_mask = host.masks[0].id;
    let vegas = effect(host, "vegas");
    assert_eq!(
        vegas.param("path"),
        Some(&EffectValue::MaskPath(Some(first_mask)))
    );
    assert_eq!(
        vegas.param("source"),
        Some(&EffectValue::Choice(2)),
        "Stroke: Mask/Path is Lumit's mask source"
    );
    // Eight segments round that mask's own perimeter.
    assert_close(
        float(vegas, "segment_length"),
        perimeter(&host.masks[0].path) / 8.0,
    );
    assert_close(float(vegas, "width"), 6.0);
    assert_close(float(vegas, "opacity"), 1.0 * 100.0);
    assert_close(colour(vegas, "colour")[0], 1.0);

    let scribble = effect(host, "scribble");
    assert_eq!(
        scribble.param("path"),
        Some(&EffectValue::MaskPath(Some(first_mask)))
    );
    assert_close(float(scribble, "angle"), 45.0);
    assert_close(float(scribble, "stroke_width"), 4.0);
    assert_close(float(scribble, "opacity"), 1.0 * 100.0);
    assert_eq!(
        scribble.param("wiggle_type"),
        Some(&EffectValue::Choice(2)),
        "AE's third wiggle type is Lumit's third"
    );
    assert_eq!(scribble.param("seed"), Some(&EffectValue::Seed(1)));
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// **The report's counts, and the placeholder the fixture is built to
/// produce.**
///
/// Curves is the effect After Effects' own scripting cannot read: its point
/// list is `CUSTOM_VALUE` data (K-410), so the instance keeps its slot as a
/// placeholder *and* the property is named as unreadable — the pair, not one
/// or the other, is what stops a Curves shipping with no curve.
///
/// The fixture's `ADBE Invert` used to be the second half of that story, as a
/// match name the table did not carry. It carries one now, and the other end of
/// the same rule is what the fixture proves instead: its Channel is Red, one of
/// the twelve After Effects has and Lumit does not, so the effect imports and
/// the *control* is reported as approximated rather than the picture quietly
/// changing (docs/11 §5).
#[test]
fn the_report_counts_what_it_says_and_names_its_placeholder() {
    let report = report();

    assert_eq!(
        report.summary(),
        Summary {
            imported: 62,
            // One fewer Adjusted row since K-497: the guide flag is a switch
            // Lumit has now, so it crosses over instead of being reported.
            // One more since Invert joined the table: the fixture's Channel is
            // Red, which is a row rather than a placeholder now. One more
            // since K-625: the fixture's camera is a two-node one, and the
            // point of interest that aims it is named rather than dropped.
            // Two fewer since K-666: the Transform effect has a Skew pair now,
            // so the fixture's Skew and Skew Axis carry rather than report.
            adjusted: 58,
            placeholders: 1,
            skipped: 1,
        }
    );

    let host = layer(fixture(), "fx host");
    let curves = effect(host, "ADBE CurvesCustom");
    assert_eq!(curves.effect.namespace, EffectNamespace::Placeholder);
    assert_eq!(curves.custom_name.as_deref(), Some("Curves"));
    assert!(curves.enabled);
    assert!(reported(|r| matches!(
        r,
        Reason::EffectPlaceholder { match_name } if match_name == "ADBE CurvesCustom"
    )));
    assert!(reported(|r| matches!(
        r,
        Reason::PropertyUnreadable { match_name } if match_name == "ADBE CurvesCustom-0001"
    )));

    // Invert maps, and the one control Lumit has no counterpart for is a row.
    let invert = effect(host, "invert");
    assert_eq!(invert.effect.namespace, EffectNamespace::Builtin);
    // Blend With Original 0 is the whole effect, which is a Mix of 100.
    assert_eq!(
        invert.param("mix"),
        Some(&EffectValue::Float(lumit_core::anim::Property::fixed(
            100.0
        )))
    );
    assert!(reported(|r| matches!(
        r,
        Reason::EffectParamApproximated { effect, param, .. }
            if effect == "Invert" && param == "Channel"
    )));

    // Exactly that one, and exactly one unreadable property.
    assert_eq!(report.of(Outcome::Placeholder).len(), 1);
    assert_eq!(report.of(Outcome::Skipped).len(), 1);

    // Every row reads as a sentence naming where it happened.
    for row in &report.rows {
        let said = row.to_string();
        assert!(said.contains(": "), "a row names where it happened: {said}");
    }
}

/// **The capture's hundred and nine unreadables are the classes we know
/// about — and a new class fails this test.**
///
/// After Effects' scripting DOM refuses four kinds of property, and every one
/// of the 109 rows in the golden bundle's `report.json` is one of them:
///
/// - **the gradient blobs** — every layer carries a Layer Styles group whose
///   Outer Glow, Inner Glow and Gradient Overlay each hold a colour ramp the
///   DOM will not hand over, three per layer across the 22 layers that have
///   the group, plus the shape layer's own gradient fill;
/// - **`ADBE Layer Source Alternate`** — the Source Options row every footage,
///   solid and precomp layer carries;
/// - **the three `CUSTOM_VALUE` blobs** the impl note §3 names by hand:
///   Curves' point list, Levels' histogram, Hue/Saturation's channel ranges;
/// - **the effects' own hidden group rows** — Fractal Noise's six, Vegas'
///   eight, Scribble's four and Tint's one, which are section headers in the
///   Effect Controls panel and `NO_VALUE` properties in the DOM.
///
/// The assertion is the exact tally rather than a spot check, because the
/// failure worth catching is a *new* kind of refusal appearing — that would
/// mean the walker started asking for something it did not ask for before, and
/// a quietly growing unreadable list is how an import loses data.
#[test]
fn the_captures_unreadables_are_the_four_classes_we_know_about() {
    let unreadables = &golden().0.report.unreadables;
    assert_eq!(unreadables.len(), 109);

    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for row in unreadables {
        *tally
            .entry(row.match_name.as_deref().unwrap_or("(unnamed)"))
            .or_default() += 1;
    }

    let expected: BTreeMap<&str, usize> = [
        // The gradient blobs: three per layer that has Layer Styles, plus the
        // shape layer's gradient fill.
        ("outerGlow/gradient", 22),
        ("innerGlow/gradient", 22),
        ("gradientFill/gradient", 22),
        ("ADBE Vector Grad Colors", 1),
        // The Source Options row.
        ("ADBE Layer Source Alternate", 20),
        // The three the impl note names by hand.
        ("ADBE CurvesCustom-0001", 1),
        ("ADBE Easy Levels2-0002", 1),
        ("ADBE HUE SATURATION-0003", 1),
        // Fractal Noise's hidden group rows.
        ("ADBE Fractal Noise-0007", 1),
        ("ADBE Fractal Noise-0014", 1),
        ("ADBE Fractal Noise-0016", 1),
        ("ADBE Fractal Noise-0022", 1),
        ("ADBE Fractal Noise-0024", 1),
        ("ADBE Fractal Noise-0028", 1),
        // And the same shape on the other three effects that have them.
        ("APC Vegas-0027", 1),
        ("APC Vegas-0033", 1),
        ("APC Vegas-0035", 1),
        ("APC Vegas-0049", 1),
        ("APC Vegas-0051", 1),
        ("APC Vegas-0054", 1),
        ("APC Vegas-0056", 1),
        ("APC Vegas-0069", 1),
        ("ADBE Scribble Fill-0009", 1),
        ("ADBE Scribble Fill-0021", 1),
        ("ADBE Scribble Fill-0031", 1),
        ("ADBE Scribble Fill-0045", 1),
        ("ADBE Tint-0004", 1),
    ]
    .into_iter()
    .collect();

    assert_eq!(
        tally, expected,
        "a class of unreadable this test has never seen — the walker is asking \
         After Effects for something new"
    );

    // Only the three CUSTOM_VALUE rows say so; the rest are NO_VALUE headers.
    let custom = unreadables
        .iter()
        .filter(|u| {
            u.error
                .as_deref()
                .is_some_and(|e| e.contains("CUSTOM_VALUE"))
        })
        .count();
    assert_eq!(custom, 3);
}

// ---------------------------------------------------------------------------
// The round trip
// ---------------------------------------------------------------------------

/// **The golden document survives a save and a reload.**
///
/// The whole point of importing into an ordinary [`Document`]: `lumit-project`
/// carries it — placeholders, `ae` namespaces, negative spans and all — with
/// no second dialect of the Lumit format to maintain (K-410). A sample of the
/// deepest assertions is re-run on the reloaded document rather than a shallow
/// count, because the shapes worth doubting are the ones only an import makes:
/// a keyframe's ease, a matte reference, a converted effect parameter, an
/// expression, and a Retime that runs backwards through negative layer time.
#[test]
fn the_golden_document_round_trips_through_a_saved_project() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("golden.lum");
    lumit_project::save(doc(), &file).expect("it saves");
    let (back, _) = lumit_project::open(&file).expect("and opens");

    assert_eq!(back.items.len(), doc().items.len());
    let c = comp(&back, "Fixture");
    assert_eq!(c.layers.len(), 23);

    // A keyframe ease.
    let x = keys(&layer(c, "child A").transform.position_x);
    assert_eq!(
        x[0].interp_out,
        SideInterp::Bezier {
            speed: 0.0,
            influence: 0.75
        }
    );
    assert_eq!(x[3].interp_out, SideInterp::Hold);

    // A matte.
    let matte = layer(c, "alpha matted").matte.as_ref().expect("a matte");
    assert_eq!(matte.layer, layer(c, "alpha matte source").id);
    assert_eq!(matte.channel, MatteChannel::Alpha);
    assert!(matte.inverted);

    // A converted effect parameter, and a placeholder's whole dump.
    let host = layer(c, "fx host");
    assert_close(
        float(effect(host, "drop_shadow"), "opacity"),
        180.0 * 100.0 / 255.0,
    );
    let curves = effect(host, "ADBE CurvesCustom");
    assert_eq!(curves.effect.namespace, EffectNamespace::Placeholder);
    assert!(curves.extra.get("ae").is_some(), "the dump survives");

    // An expression, and the reversed Retime with its negative layer time.
    assert_eq!(
        layer(c, "child A").transform.opacity.animation,
        Animation::Expression("50 + 25".to_string())
    );
    let reversed = layer(c, "reversed");
    assert!(reversed.in_point < reversed.out_point);
    assert_close(
        reversed.retime.as_ref().expect("a Retime").value_at(-5.0),
        5.0,
    );
}

/// **The twenty-two layers carrying the Layer Styles group wear no style, and
/// the import puts none on them** (K-706, docs/impl/layer-styles.md §7).
///
/// After Effects lists all ten style slots on any layer that has ever had the
/// group, switched off or not, and every one of the bundle's two hundred and
/// twenty slots is off — nobody in the fixture project actually dressed a
/// layer. So the map stage's own rule is what this pins: an off slot is a style
/// nobody added, and importing them would put eighty disabled instances on
/// layers that show none in After Effects.
///
/// The positive half — the angle formula, the opacity, the order and every
/// report row — is in `src/map/styles/tests.rs`, against hand-built groups, so
/// each can be pinned exactly rather than hunted for in a megabyte of JSON.
/// What lives here is the fact only the real capture can prove.
#[test]
fn the_bundles_layer_styles_are_all_switched_off_and_none_import() {
    let mut groups = 0usize;
    let mut slots = 0usize;
    fn walk(props: &[lumit_import::capture::Property], groups: &mut usize, slots: &mut usize) {
        for node in props {
            if node.match_name.as_deref() == Some("ADBE Layer Styles") {
                *groups += 1;
                for style in node.children() {
                    if style.match_name.as_deref() != Some("ADBE Blend Options Group") {
                        *slots += 1;
                        assert_eq!(
                            style.enabled,
                            Some(false),
                            "a style switched on would give this layer one, and the counts below \
                             would need saying differently"
                        );
                    }
                }
            }
            walk(node.children(), groups, slots);
        }
    }
    for comp in &golden().0.capture.comps {
        for layer in &comp.layers {
            walk(&layer.properties, &mut groups, &mut slots);
        }
    }
    assert_eq!(groups, 22, "the layers carrying the group");
    assert_eq!(slots, 220, "ten slots apiece");

    for item in &doc().items {
        if let ProjectItem::Composition(comp) = item {
            for layer in &comp.layers {
                assert!(
                    layer.styles.is_empty(),
                    "{} wears a style nobody switched on",
                    layer.name
                );
            }
        }
    }

    // And no style row reached the report, which is what "nothing to say"
    // looks like: the gradient ramps the DOM refuses are still counted by the
    // capture, and the map stage never visited them.
    assert!(!report().rows.iter().any(|row| matches!(
        &row.reason,
        Reason::EffectParamNotCarried { effect, .. } if effect == "Layer styles"
    )));
}
