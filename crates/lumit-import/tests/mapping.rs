//! The structural mapping, end to end: a capture becomes a document
//! (docs/11-AE-IMPORT.md, docs/impl/ae-import.md §4, §6).
//!
//! Two fixtures feed these tests, and the split is deliberate.
//! `synthetic.lum-bundle` is the *ordinary* half of an After Effects
//! project — the things that map — and `edges.lum-bundle` is the awkward half:
//! the blend modes with no equivalent, the layer kinds the fidelity matrix
//! grades below lossless, and the four ways a capture can be damaged. Between
//! them every bullet of the mapping has an assertion, and the second one exists
//! mostly to prove the standing rule: **an import never fails**.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lumit_core::anim::{Animation, SideInterp};
use lumit_core::mask::MaskMode;
use lumit_core::model::{
    BlendMode, Composition, Document, EffectNamespace, EffectValue, Layer, LayerKind, LightKind,
    MatteChannel, ProjectItem,
};
use lumit_core::retime::Interpolation;
use lumit_core::time::Rational;
use lumit_import::{map_capture, ImportReport, Outcome, Reason};

fn mapped(fixture: &str) -> (Document, ImportReport) {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture);
    let bundle = lumit_import::open_bundle(&path).expect("the fixture opens");
    map_capture(&bundle.capture)
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

/// Whether any report row's reason satisfies `f` — the shape most assertions
/// here take, because a row's *path* is prose and its reason is the fact.
fn reported(report: &ImportReport, f: impl Fn(&Reason) -> bool) -> bool {
    report.rows.iter().any(|row| f(&row.reason))
}

fn keys(property: &lumit_core::anim::Property) -> Vec<lumit_core::anim::Keyframe> {
    match &property.animation {
        Animation::Keyframed(keys) => keys.clone(),
        other => panic!("not keyframed: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The ordinary half
// ---------------------------------------------------------------------------

/// **The project panel arrives whole: the folder tree, the items, and the
/// compositions.**
///
/// The join this proves is the one the capture schema makes on purpose and an
/// importer gets wrong first: a composition carries no name of its own, so it
/// is only ever identified by an id shared with the item list, and the folder
/// tree is rebuilt from `parent_id` rather than captured as a tree. Getting
/// either wrong gives a Project panel that is *plausible* and wrong, which is
/// worse than an empty one.
#[test]
fn the_item_tree_is_rebuilt_from_parent_ids() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    assert_eq!(doc.items.len(), 5);

    let assets = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Folder(f) if f.name == "Assets" => Some(f),
            _ => None,
        })
        .expect("the Assets folder");
    assert_eq!(
        assets.children.len(),
        3,
        "footage, solid, and the nested comp"
    );
    assert!(assets.children.contains(&item_id(&doc, "clip.mp4")));
    assert!(assets.children.contains(&item_id(&doc, "Nested")));

    // Main sat at the root, so it is not anybody's child.
    let roots = doc.root_items();
    assert!(roots.contains(&item_id(&doc, "Main")));
    assert!(roots.contains(&assets.id));
    assert!(!roots.contains(&item_id(&doc, "clip.mp4")));

    // The footage's path is carried; relink is a later phase, and an item's
    // interpretation settings ride in the `ae` namespace until Lumit has
    // fields for them.
    let footage = match doc.item(item_id(&doc, "clip.mp4")) {
        Some(ProjectItem::Footage(f)) => f,
        other => panic!("expected footage, got {other:?}"),
    };
    assert_eq!(footage.media.relative_path, "/media/clip.mp4");
    let ae = footage.extra.get("ae").expect("an ae namespace");
    assert_eq!(ae.get("id"), Some(&serde_json::json!(2)));
    assert_eq!(ae.get("alpha"), Some(&serde_json::json!("PREMULTIPLIED")));
    assert_eq!(ae.get("fps_override"), Some(&serde_json::json!(25.0)));
}

/// **A composition's settings, its shutter, and its markers.**
///
/// Motion blur is the one comp setting where the two applications agree on
/// every unit — angle in degrees, phase in degrees, samples per frame — so a
/// mistake here would be a plain transcription slip, which is exactly the kind
/// a test catches and a reading does not.
#[test]
fn a_composition_keeps_its_settings_shutter_and_markers() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    let main = comp(&doc, "Main");

    assert_eq!((main.width, main.height), (1920, 1080));
    assert_eq!((main.frame_rate.num(), main.frame_rate.den()), (25, 1));
    assert_eq!(main.duration.0, Rational::new(10, 1).unwrap());
    assert!(main.motion_blur.enabled);
    assert_eq!(main.motion_blur.shutter_angle, 180.0);
    assert_eq!(main.motion_blur.shutter_phase, -90.0);
    assert_eq!(main.motion_blur.samples, 16);

    assert_eq!(main.markers.len(), 1);
    assert_eq!(main.markers[0].label, "chorus");
    assert_eq!(main.markers[0].time.0, Rational::new(2, 1).unwrap());
    assert_eq!(main.markers[0].duration, Some(Rational::new(1, 2).unwrap()));

    // The renderer has no Lumit meaning but names what a comp relied on.
    assert_eq!(
        main.extra.get("ae").and_then(|ae| ae.get("renderer")),
        Some(&serde_json::json!("ADBE Advanced 3d"))
    );

    // A layer marker rides on the layer, not the ruler.
    let clip = layer(main, "clip.mp4");
    assert_eq!(clip.markers.len(), 1);
    assert_eq!(clip.markers[0].label, "hit");
}

/// **Stacking order, kinds, parenting and every switch that has a
/// counterpart.**
///
/// Parenting is by *index* in the capture and by id in the document, and the
/// index can point anywhere in the stack — including at a layer that has not
/// been built yet — which is why the ids are handed out before anything is
/// mapped. A parent resolved to the wrong row is a rig that moves the wrong
/// things.
#[test]
fn the_layer_stack_keeps_its_order_kinds_parenting_and_switches() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    let main = comp(&doc, "Main");

    let names: Vec<&str> = main.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Nested", "clip.mp4", "Black Solid 1"], "top first");

    let precomp = layer(main, "Nested");
    assert_eq!(
        precomp.kind,
        LayerKind::Precomp {
            comp: item_id(&doc, "Nested")
        }
    );
    assert_eq!(precomp.blend, BlendMode::Screen);
    assert!(precomp.switches.collapse);
    assert!(precomp.switches.motion_blur);
    assert_eq!(precomp.label, 8);

    let clip = layer(main, "clip.mp4");
    assert_eq!(
        clip.kind,
        LayerKind::Footage {
            item: item_id(&doc, "clip.mp4")
        }
    );
    assert_eq!(clip.parent, Some(layer(main, "Black Solid 1").id));
    assert!(clip.switches.shy);
    assert_eq!(clip.in_point.0, Rational::ZERO);
    assert_eq!(clip.out_point.0, Rational::new(8, 1).unwrap());

    let solid = layer(main, "Black Solid 1");
    assert_eq!(
        solid.kind,
        LayerKind::Solid {
            def: item_id(&doc, "Black Solid 1")
        }
    );
    // The video switch of a layer used as somebody's matte is preserved as it
    // stood in After Effects (docs/11 §3), not forced back on.
    assert!(!solid.switches.visible);
    assert!(solid.switches.locked);
}

/// **A keyframed curve is copied, not recomputed** (K-025).
///
/// The three things that would each silently ruin an animation: reading one
/// interpolation type per key rather than per *side* (every hold key becomes a
/// ramp), reading a single ease rather than the per-dimension array, and
/// forgetting that After Effects' influence is a percentage where Lumit's is a
/// fraction (every curve comes out a hundred times too eager).
#[test]
fn a_bezier_and_a_hold_key_come_across_side_by_side() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    let clip = layer(comp(&doc, "Main"), "clip.mp4");
    let blur = clip
        .effects
        .iter()
        .find(|e| e.effect.match_name == "blur")
        .expect("the blur instance");
    let EffectValue::Float(radius) = blur.param("radius").expect("the Radius parameter") else {
        panic!("Radius is a float");
    };

    // Blurriness is raster pixels in After Effects and a per cent of the comp
    // diagonal in Lumit (docs/08 §2.3), so the values and the handle speeds —
    // which are value-units a second — arrive multiplied by the same factor.
    let k = 100.0 / (1920.0f64 * 1920.0 + 1080.0 * 1080.0).sqrt();

    let keys = keys(radius);
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[0].time, Rational::ZERO);
    let SideInterp::Bezier { speed, influence } = keys[0].interp_out else {
        panic!("an eased out-side");
    };
    assert_eq!(speed, 0.0);
    assert!(
        (influence - 0.33333333).abs() < 1e-12,
        "AE's 33.333333% is Lumit's 0.33333333, not {influence}"
    );
    let SideInterp::Bezier { speed, influence } = keys[1].interp_in else {
        panic!("an eased in-side");
    };
    assert!((speed - 120.0 * k).abs() < 1e-12);
    assert!((influence - 0.5).abs() < 1e-12);
    assert_eq!(
        keys[1].interp_out,
        SideInterp::Hold,
        "a hold key is a hold key"
    );
    assert_eq!(keys[2].time, Rational::new(4, 1).unwrap());

    // And it evaluates as a hold: flat across the whole span.
    assert!((radius.value_at(3.0) - 40.0 * k).abs() < 1e-12);
}

/// **A separated position animates per axis, and the leader is not the
/// animation.**
///
/// After Effects' own trap. The leader of a separated property still reports a
/// still value, so reading it is not an error anybody sees — it is a moving
/// layer that quietly stopped moving.
#[test]
fn a_separated_position_animates_on_its_followers() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    let clip = layer(comp(&doc, "Main"), "clip.mp4");

    let x = keys(&clip.transform.position_x);
    assert_eq!(x.len(), 2);
    assert_eq!(x[0].value, 100.0);
    assert_eq!(x[1].value, 1820.0);
    assert_eq!(x[1].time, Rational::new(2, 1).unwrap());
    assert_eq!(
        x[0].interp_out,
        SideInterp::Bezier {
            speed: 0.0,
            influence: 0.75
        }
    );

    // The Y follower had no keys of its own, so it is the still it was.
    assert_eq!(
        clip.transform.position_y.animation,
        Animation::Static(540.0)
    );
}

/// **A coupled spatial property splits into lanes, and says the motion path
/// did not come with it.**
///
/// Lumit animates each axis on its own, so AE's spatial tangents have nowhere
/// to go. The *values* are exact either way — this is a report row, not a
/// silent loss.
#[test]
fn a_coupled_position_splits_into_lanes_and_reports_its_tangents() {
    let (doc, report) = mapped("synthetic.lum-bundle");
    let precomp = layer(comp(&doc, "Main"), "Nested");

    let x = keys(&precomp.transform.position_x);
    let y = keys(&precomp.transform.position_y);
    assert_eq!((x[0].value, y[0].value), (480.0, 540.0));
    assert_eq!((x[1].value, y[1].value), (1440.0, 620.0));
    assert_eq!(x[1].time, Rational::new(3, 1).unwrap());
    assert!(reported(&report, |r| matches!(
        r,
        Reason::SpatialTangentsFlattened
    )));
}

/// **An expression drives the property; a switched-off one is kept and does
/// not.**
///
/// Exactly After Effects' own behaviour, and the reason both states have to
/// survive: a disabled expression's text is still there to be switched back
/// on, and it must not be quietly promoted into driving the layer on the way
/// across.
#[test]
fn an_expression_drives_the_property_only_when_it_was_switched_on() {
    let (doc, report) = mapped("synthetic.lum-bundle");

    let clip = layer(comp(&doc, "Main"), "clip.mp4");
    assert_eq!(
        clip.transform.opacity.animation,
        Animation::Expression("wiggle(2, 30)".to_string())
    );
    assert!(reported(&report, |r| matches!(
        r,
        Reason::ExpressionCarried
    )));

    let solid = layer(comp(&doc, "Nested"), "White Solid 1");
    assert_eq!(solid.transform.rotation.animation, Animation::Static(0.0));
    assert_eq!(
        solid
            .transform
            .rotation
            .extra
            .get("ae")
            .and_then(|ae| ae.get("expression")),
        Some(&serde_json::json!("time * 45")),
        "the text is kept so it can be switched on later"
    );
    assert!(reported(&report, |r| matches!(
        r,
        Reason::ExpressionDisabledCarried
    )));
}

/// **A matte is normalised to a chosen layer, a channel, and an inversion.**
///
/// The 23.0+ selectable form, which names its layer outright.
#[test]
fn a_selectable_matte_normalises_to_a_layer_and_a_channel() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    let main = comp(&doc, "Main");
    let matte = layer(main, "clip.mp4").matte.expect("a matte");

    assert_eq!(matte.layer, layer(main, "Black Solid 1").id);
    assert_eq!(matte.channel, MatteChannel::Alpha);
    assert!(matte.inverted, "ALPHA_INVERTED is Alpha plus inverted");
}

/// **A mask keeps its mode, its inversion, its feather and its path.**
///
/// The path is the only property value with structure rather than numbers, so
/// it is the one whose conversion is worth proving vertex by vertex: AE hands
/// over three parallel arrays and Lumit stores one vertex carrying both its
/// tangents.
#[test]
fn a_mask_keeps_its_mode_inversion_feather_and_path() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    let clip = layer(comp(&doc, "Main"), "clip.mp4");
    assert_eq!(clip.masks.len(), 1);
    let mask = &clip.masks[0];

    assert_eq!(mask.name, "Mask 1");
    assert_eq!(mask.mode, MaskMode::Subtract);
    assert!(mask.inverted);
    assert_eq!(mask.feather.value_at(0.0), 12.0);
    assert_eq!(mask.opacity.value_at(0.0), 100.0);
    assert_eq!(mask.expansion.value_at(0.0), 0.0);

    assert_eq!(mask.path.vertices.len(), 4);
    assert!(mask.path.closed);
    assert_eq!(mask.path.vertices[1].pos, (100.0, 0.0));
    assert_eq!(mask.path.vertices[1].tan_in, (-20.0, 0.0));
    assert_eq!(mask.path.vertices[0].tan_out, (20.0, 0.0));
}

/// **A time remap becomes a Retime, and its hold key becomes a freeze.**
///
/// The two graphs are the same mathematical object (docs/11 §2.2 item 6), so
/// this is a value copy and the freeze needs nothing translating it: a held
/// span returns the held source time, which is a frozen frame by definition.
/// The frame-blending switch is a separate control and lands on the
/// interpolation policy beside the map, never inside it (docs/04 §10).
#[test]
fn a_time_remap_becomes_a_retime_whose_hold_key_is_a_freeze() {
    let (doc, report) = mapped("synthetic.lum-bundle");
    let precomp = layer(comp(&doc, "Main"), "Nested");
    let retime = precomp.retime.as_ref().expect("a Retime");

    let keys = keys(retime);
    assert_eq!(keys.len(), 2);
    assert_eq!((keys[0].time, keys[0].value), (Rational::ZERO, 0.0));
    assert_eq!(keys[1].time, Rational::new(5, 1).unwrap());
    assert_eq!(keys[1].value, 2.5);
    assert_eq!(keys[1].interp_out, SideInterp::Hold);

    // Past the freeze, the same source moment for ever.
    assert_eq!(retime.value_at(6.0), 2.5);
    assert_eq!(retime.value_at(9.9), 2.5);

    assert!(matches!(precomp.interpolation, Interpolation::Flow(_)));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::FlowEngineDiffers
    )));

    // Frame Mix is the plain crossfade; no frame blending is nearest.
    assert_eq!(
        layer(comp(&doc, "Main"), "clip.mp4").interpolation,
        Interpolation::Blend
    );
    assert_eq!(
        layer(comp(&doc, "Main"), "Black Solid 1").interpolation,
        Interpolation::Nearest
    );
}

/// **A time stretch becomes the Retime that says the same thing.**
///
/// Lumit has no stretch switch, and does not need one: a layer at 50% plays
/// its source twice as fast, which is a straight line of slope two from layer
/// time to source time.
#[test]
fn a_time_stretch_becomes_the_equivalent_retime() {
    let (doc, report) = mapped("synthetic.lum-bundle");
    let clip = layer(comp(&doc, "Main"), "clip.mp4");
    let retime = clip.retime.as_ref().expect("a Retime from the stretch");

    assert_eq!(retime.value_at(0.0), 0.0);
    assert_eq!(retime.value_at(4.0), 8.0, "50% stretch is double speed");
    assert_eq!(retime.value_at(8.0), 16.0);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::StretchAsRetime { percent } if (percent - 50.0).abs() < 1e-9
    )));
}

/// **An effect the table does not know imports as a placeholder that keeps
/// everything, and one it does know imports as the Lumit effect.**
///
/// docs/11 §5 and §6, the two halves of the effect story side by side: an
/// unmapped match name becomes an inert node holding the display name, the
/// on/off state and every animatable parameter as a real Lumit property that
/// animates and shows in the graph editor — never the closest guess — while a
/// mapped one becomes the built-in that does the same job.
#[test]
fn an_unmapped_effect_imports_as_a_placeholder_keeping_its_parameters() {
    let (doc, report) = mapped("synthetic.lum-bundle");
    let clip = layer(comp(&doc, "Main"), "clip.mp4");
    assert_eq!(
        clip.effects.len(),
        2,
        "Curves and the blur both keep a slot"
    );

    // The Gaussian blur is in the table (docs/11 §5), so it is the built-in.
    let blur = &clip.effects[1];
    assert_eq!(blur.effect.namespace, EffectNamespace::Builtin);
    assert_eq!(blur.effect.match_name, "blur");
    assert!(blur.enabled);

    // Curves' point list is the one property After Effects itself cannot read
    // (K-410), so the effect keeps its slot and the report names the property
    // rather than shipping a Curves with no curve.
    let curves = &clip.effects[0];
    assert_eq!(curves.effect.namespace, EffectNamespace::Placeholder);
    assert_eq!(curves.effect.match_name, "ADBE CurvesCustom");
    assert_eq!(curves.custom_name.as_deref(), Some("Curves"));
    assert!(curves.params.is_empty());
    let carried = curves
        .extra
        .get("ae")
        .and_then(|ae| ae.get("params"))
        .and_then(|p| p.as_array())
        .expect("the unmappable leaves are kept whole");
    assert_eq!(carried.len(), 1);
    assert_eq!(
        carried[0].get("match_name"),
        Some(&serde_json::json!("ADBE CurvesCustom-0001"))
    );
    assert!(reported(&report, |r| matches!(
        r,
        Reason::PropertyUnreadable { match_name } if match_name == "ADBE CurvesCustom-0001"
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::EffectPlaceholder { match_name } if match_name == "ADBE CurvesCustom"
    )));
}

/// **A placeholder's parameters are real Lumit properties: they animate, and
/// what has no property shape is kept verbatim.**
///
/// docs/11 §6 in full, on a third-party effect that will never be in the table
/// (K-060): the keyframed Speed becomes a keyframed property, the colour
/// becomes a colour, the unreadable blob and the layer reference are kept in
/// the `ae` namespace, and the whole thing is switched off exactly as it was.
#[test]
fn a_placeholders_parameters_animate_and_nothing_is_dropped() {
    let (doc, report) = mapped("edges.lum-bundle");
    let third = layer(comp(&doc, "Edges"), "Third party");
    let fx = &third.effects[0];

    assert_eq!(fx.effect.namespace, EffectNamespace::Placeholder);
    assert_eq!(fx.effect.match_name, "RE:Vision Twixtor");
    assert_eq!(fx.custom_name.as_deref(), Some("Twixtor Pro"));
    assert!(!fx.enabled, "a switched-off effect imports switched off");

    let EffectValue::Float(speed) = fx
        .param("RE:Vision Twixtor-0001")
        .expect("the Speed parameter")
    else {
        panic!("Speed is a float");
    };
    assert_eq!(keys(speed).len(), 2);
    assert_eq!(speed.value_at(2.0), 25.0);
    assert!(matches!(
        fx.param("RE:Vision Twixtor-0002"),
        Some(EffectValue::Colour(_))
    ));

    let carried = fx
        .extra
        .get("ae")
        .and_then(|ae| ae.get("params"))
        .and_then(|p| p.as_array())
        .expect("the unmappable leaves are kept whole");
    assert_eq!(carried.len(), 2, "the blob and the layer reference");
    assert!(reported(&report, |r| matches!(
        r,
        Reason::PropertyUnreadable { match_name } if match_name == "RE:Vision Twixtor-0003"
    )));
}

// ---------------------------------------------------------------------------
// The awkward half
// ---------------------------------------------------------------------------

/// **A blend mode with no equivalent falls back to Normal and says so; a
/// Classic one becomes its modern namesake.**
///
/// docs/11 §4's two mapped blend rows. The picture changes either way, so the
/// only unacceptable outcome is a quiet one.
#[test]
fn an_unavailable_blend_mode_falls_back_and_a_classic_one_modernises() {
    let (doc, report) = mapped("edges.lum-bundle");
    let edges = comp(&doc, "Edges");

    assert_eq!(layer(edges, "Dissolve").blend, BlendMode::Normal);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::BlendModeUnavailable { ae_mode } if ae_mode == "DISSOLVE"
    )));

    assert_eq!(layer(edges, "Classic").blend, BlendMode::ColourBurn);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::BlendModeClassic { ae_mode } if ae_mode == "CLASSIC_COLOR_BURN"
    )));

    // A name from a future After Effects is not a reason to fail.
    assert_eq!(layer(edges, "Orphan").blend, BlendMode::Normal);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::BlendModeUnavailable { ae_mode } if ae_mode == "MADE_UP_MODE"
    )));
}

/// **A legacy matte resolves to the layer above.**
///
/// The older After Effects form says only "I have a matte"; which layer is
/// implied by the stack. Resolving it here is what lets both generations
/// arrive as one thing.
#[test]
fn a_legacy_matte_resolves_to_the_layer_above() {
    let (doc, _) = mapped("edges.lum-bundle");
    let edges = comp(&doc, "Edges");
    let matte = layer(edges, "Legacy matte user").matte.expect("a matte");

    assert_eq!(matte.layer, layer(edges, "Reversed").id, "the layer above");
    assert_eq!(matte.channel, MatteChannel::Luma);
    assert!(!matte.inverted);
}

/// **A negative stretch plays the layer backwards.**
///
/// One multiplication, the same as a positive stretch: source time is layer
/// time times the rate. After Effects has already done the turning round —
/// reversing a layer moves its own zero (`start_time`) to the *far end* of the
/// bar, so the layer's local time runs from −2 up to 0 and multiplying by −1
/// walks the source from its last moment back to its first.
#[test]
fn a_negative_stretch_plays_the_layer_backwards() {
    let (doc, report) = mapped("edges.lum-bundle");
    let reversed = layer(comp(&doc, "Edges"), "Reversed");
    let retime = reversed.retime.as_ref().expect("a Retime");

    assert_eq!(reversed.start_offset.0, Rational::new(2, 1).unwrap());
    assert_eq!(retime.value_at(-2.0), 2.0, "opens on the far end");
    assert_eq!(retime.value_at(-1.0), 1.0);
    assert_eq!(retime.value_at(0.0), 0.0, "and walks back to the start");
    assert!(reported(&report, |r| matches!(
        r,
        Reason::StretchAsRetime { percent } if (percent + 100.0).abs() < 1e-9
    )));
}

/// **A key that is not on a frame is not moved onto one.**
///
/// The impl note's §4 rule, and the one whose failure is invisible: snapping a
/// half-frame key rounds somebody's animation onto the grid and nothing in the
/// picture says it happened.
#[test]
fn an_off_frame_key_stays_off_the_frame() {
    let (doc, _) = mapped("edges.lum-bundle");
    let edges = comp(&doc, "Edges");
    let x = keys(&layer(edges, "Off frame").transform.position_x);

    let frame = |n: i64| edges.frame_rate.time_of_frame(n).unwrap().0;
    assert_eq!(x[0].time, frame(0));
    assert!(
        x[1].time > frame(12) && x[1].time < frame(13),
        "between two frames"
    );
    assert!(
        (x[1].time.to_f64() - 0.5213541666666667).abs() < 1e-5,
        "and within the sub-frame grid of where it was"
    );

    // The composition's rate came back as the fraction, not the decimal.
    assert_eq!(
        (edges.frame_rate.num(), edges.frame_rate.den()),
        (24000, 1001)
    );
}

/// **Key times are measured from the layer's own start, not the
/// composition's.**
///
/// After Effects reports a layer property's key times on the composition's
/// clock; Lumit stores them on the layer's, which begins at its start offset.
/// A layer dragged two seconds down the timeline would otherwise import with
/// its animation two seconds into the future.
#[test]
fn key_times_are_measured_from_the_layers_own_start() {
    let (doc, _) = mapped("edges.lum-bundle");
    let shifted = layer(comp(&doc, "Edges"), "Shifted");
    assert_eq!(shifted.start_offset.0, Rational::new(2, 1).unwrap());

    let rotation = keys(&shifted.transform.rotation);
    assert_eq!(
        rotation[0].time,
        Rational::ZERO,
        "layer time, not comp time"
    );
    assert_eq!(rotation[1].time, Rational::new(1, 1).unwrap());
    assert_eq!(shifted.transform.rotation.value_at(0.0), 0.0);
    assert_eq!(shifted.transform.rotation.value_at(1.0), 90.0);
}

/// **An unbuilt mask mode falls back to Add, and an animated path keeps its
/// keys.**
///
/// Lighten and Darken are not built (docs/06 §2). Everything else about the
/// mask — feather, opacity, expansion, the animated shape — comes across, and
/// AE's two feather axes average into Lumit's one width with a row saying so.
#[test]
fn an_unbuilt_mask_mode_falls_back_and_the_animated_path_survives() {
    let (doc, report) = mapped("edges.lum-bundle");
    let mask = &layer(comp(&doc, "Edges"), "Lighten mask").masks[0];

    assert_eq!(mask.mode, MaskMode::Add);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::MaskModeUnavailable { ae_mode } if ae_mode == "LIGHTEN"
    )));

    assert_eq!(mask.feather.value_at(0.0), 7.0, "10 and 4 average to 7");
    assert!(reported(&report, |r| matches!(
        r,
        Reason::MaskFeatherAxesDiffer { .. }
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::MaskRotoBezierFlattened
    )));

    assert_eq!(mask.opacity.value_at(0.0), 60.0);
    assert_eq!(mask.expansion.value_at(0.0), -3.0);
    assert_eq!(mask.path_keys.len(), 2);
    assert_eq!(mask.path_keys[0].interp_out, SideInterp::Hold);
    assert_eq!(mask.path_keys[1].time, Rational::new(2, 1).unwrap());
    assert_eq!(mask.path_keys[1].path.vertices[1].pos, (50.0, 0.0));
    // With no still value in the capture, the drawn shape is the first key's.
    assert_eq!(mask.path.vertices.len(), 3);
}

/// **The layer kinds Lumit has map; the rest keep their slot and are named.**
///
/// The fidelity matrix's own rule. A shape layer with no contents and a light
/// of a kind Lumit does not build are both still *layers*: they hold their
/// place in the stack, carry their transform, and can be parented to, so the
/// rig around them survives.
#[test]
fn the_layer_kinds_lumit_has_map_and_the_rest_keep_their_slot() {
    let (doc, report) = mapped("edges.lum-bundle");
    let edges = comp(&doc, "Edges");

    assert_eq!(layer(edges, "Adjust").kind, LayerKind::Adjustment);
    assert_eq!(layer(edges, "Off frame").kind, LayerKind::Null);

    let LayerKind::Camera { zoom, .. } = &layer(edges, "Cam").kind else {
        panic!("a camera");
    };
    assert_eq!(zoom.value_at(0.0), 1800.0, "read out of Camera Options");

    let LayerKind::Light { light } = &layer(edges, "Key light").kind else {
        panic!("a light");
    };
    assert_eq!(
        light.kind,
        LightKind::Point,
        "an ambient light is not a place"
    );
    assert_eq!(
        light.intensity.value_at(0.0),
        0.5,
        "AE's 50% is Lumit's 0.5"
    );
    assert_eq!(
        light.cone_deg.value_at(0.0),
        45.0,
        "AE's full cone is Lumit's half"
    );
    assert!(reported(&report, |r| matches!(
        r,
        Reason::LightKindApproximated { ae_kind } if ae_kind == "AMBIENT"
    )));

    let LayerKind::Text { document } = &layer(edges, "Words").kind else {
        panic!("a text layer");
    };
    assert_eq!(document.text, "Roll credits");
    assert_eq!(document.size, 48.0);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::TextStylingNotMapped
    )));

    assert_eq!(
        layer(edges, "Shapes").kind,
        LayerKind::Shape {
            contents: Vec::new()
        }
    );
    assert!(reported(&report, |r| matches!(
        r,
        Reason::ShapeContentsNotMapped
    )));

    // A kind this build has never heard of, and a layer whose source is gone.
    assert_eq!(layer(edges, "Weird").kind, LayerKind::Null);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::LayerKindUnsupported { ae_kind } if ae_kind == "sorcery"
    )));
    assert_eq!(layer(edges, "Orphan").kind, LayerKind::Null);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::LayerSourceMissing { id } if *id == 99
    )));
}

/// **A guide layer, a draft quality and preserve-underlying-transparency are
/// all reported rather than dropped in silence.**
///
/// Three switches with no Lumit counterpart. Each changes what a comp looks
/// like, so each is a row.
#[test]
fn the_switches_with_no_counterpart_are_reported() {
    let (doc, report) = mapped("edges.lum-bundle");
    let guide = layer(comp(&doc, "Edges"), "Guide");

    assert!(guide.switches.visible, "a guide layer imports visible");
    assert!(guide.switches.shy && guide.switches.locked && guide.switches.solo);
    assert!(guide.switches.three_d && guide.switches.collapse && guide.switches.motion_blur);
    assert!(!guide.switches.fx, "AE's fx switch is Lumit's");

    assert!(reported(&report, |r| matches!(
        r,
        Reason::GuideLayerNotSupported
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::LayerQualityIgnored { quality } if quality == "WIREFRAME"
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::PreserveTransparencyNotSupported
    )));
}

/// **Nothing damaged fails the import.**
///
/// An item with no kind, a composition the walk never described, a layer with
/// no place in the stack, a parent and a matte pointing at layers that are not
/// there, a bar with no length. Every one of them is a row and the project
/// still opens — which is the standing rule of docs/impl/ae-import.md §4 and
/// the reason this test exists at all.
#[test]
fn a_damaged_capture_skips_the_broken_parts_and_still_imports() {
    let (doc, report) = mapped("edges.lum-bundle");

    // Six of the seven items; the one with no kind was skipped.
    assert_eq!(doc.items.len(), 6);
    assert!(reported(&report, |r| matches!(r, Reason::ItemUnreadable)));

    // A comp the walk described only in part still imports, at a stated
    // default rather than at a rate of nothing per second.
    let vague = comp(&doc, "Vague");
    assert_eq!((vague.frame_rate.num(), vague.frame_rate.den()), (25, 1));
    assert_eq!(vague.duration.0, Rational::new(10, 1).unwrap());
    assert!(reported(&report, |r| matches!(
        r,
        Reason::CompFrameRateGuessed { .. }
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::CompDurationGuessed { .. }
    )));

    // An audio layer is a footage layer carrying its audio (docs/01 §2).
    assert_eq!(
        layer(vague, "Voiceover").kind,
        LayerKind::Footage {
            item: item_id(&doc, "missing.mov")
        }
    );
    assert!(reported(&report, |r| matches!(
        r,
        Reason::AudioLayerAsFootage
    )));

    // The composition nothing described still exists, so anything naming it
    // resolves.
    let ghost = comp(&doc, "Ghost");
    assert!(ghost.layers.is_empty());
    assert!(reported(&report, |r| matches!(r, Reason::CompMissing)));

    // Sixteen of the seventeen layers; the one with no index was skipped.
    let edges = comp(&doc, "Edges");
    assert_eq!(edges.layers.len(), 16);
    assert!(reported(&report, |r| matches!(r, Reason::LayerUnreadable)));

    let orphan = layer(edges, "Orphan");
    assert_eq!(orphan.parent, None);
    assert_eq!(orphan.matte, None);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::ParentMissing { index } if *index == 99
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::MatteTargetMissing { index } if *index == 99
    )));

    // A bar with no length is not a layer the model can hold.
    let weird = layer(edges, "Weird");
    assert!(weird.out_point > weird.in_point);
    assert!(reported(&report, |r| matches!(
        r,
        Reason::LayerSpanRepaired
    )));

    // And the comp-level differences that are facts rather than damage.
    assert!(reported(&report, |r| matches!(
        r,
        Reason::PixelAspectIgnored { .. }
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::CompStartIgnored { .. }
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::RendererUnrecognised { renderer } if renderer == "ADBE Ernst"
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::NestedPreserveIgnored { fps: true, .. }
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::ProjectBlendingDiffers { bits: 8 }
    )));
    assert!(reported(&report, |r| matches!(
        r,
        Reason::MediaMissing { .. }
    )));
    assert!(reported(&report, |r| matches!(r, Reason::MediaPlaceholder)));
}

/// **The report counts what it says, and every row can be read aloud.**
///
/// docs/11 §9's summary line. The counts come from the rows, so the line and
/// the list beneath it cannot disagree; and a reason is a typed fact whose
/// display is the sentence a panel shows, never something the engine parses
/// back.
#[test]
fn the_report_summarises_and_reads_as_sentences() {
    let (_, report) = mapped("edges.lum-bundle");
    let summary = report.summary();

    assert!(summary.imported > 0);
    assert!(summary.adjusted > 0);
    assert_eq!(summary.placeholders, report.of(Outcome::Placeholder).len());
    assert!(summary.skipped >= 3, "an item, a comp, and a layer");
    assert!(summary.to_string().contains("placeholders"));

    for row in &report.rows {
        let said = row.to_string();
        assert!(said.contains(": "), "a row names where it happened: {said}");
        assert!(said.len() > 20, "and says why in a sentence: {said}");
    }

    // The report serialises for `import-report.json` and the panel.
    let json = serde_json::to_string(&report).expect("the report serialises");
    let back: ImportReport = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(back, report);
}

/// **A mapped document survives a save and a reload.**
///
/// The whole point of importing into an ordinary [`Document`]: `lumit-project`
/// carries it, placeholders and `ae` namespaces included, with no second
/// dialect of the Lumit format to maintain (K-410). What this proves is that
/// nothing the mapping puts in the document is a shape the file format cannot
/// hold — the placeholder effect and the carried-through AE data being the two
/// that could plausibly have been.
#[test]
fn a_mapped_document_round_trips_through_a_saved_project() {
    let (doc, _) = mapped("synthetic.lum-bundle");
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("imported.lum");
    lumit_project::save(&doc, &file).expect("it saves");
    let (back, _) = lumit_project::open(&file).expect("and opens");

    assert_eq!(back.items.len(), doc.items.len());
    let main = comp(&back, "Main");
    assert_eq!(main.layers.len(), 3);

    let clip = layer(main, "clip.mp4");
    assert_eq!(clip.masks.len(), 1);
    assert_eq!(clip.masks[0].mode, MaskMode::Subtract);
    assert_eq!(clip.effects.len(), 2);
    assert_eq!(clip.effects[1].effect.namespace, EffectNamespace::Builtin);
    assert_eq!(clip.effects[1].effect.match_name, "blur");
    assert!(
        clip.effects[0].extra.get("ae").is_some(),
        "the dump survives"
    );
    assert_eq!(
        clip.retime.as_ref().map(|r| r.value_at(4.0)),
        Some(8.0),
        "the Retime is still the Retime"
    );
    assert_eq!(
        clip.transform.opacity.animation,
        Animation::Expression("wiggle(2, 30)".to_string())
    );
    let precomp = layer(main, "Nested");
    assert_eq!(
        precomp.kind,
        LayerKind::Precomp {
            comp: item_id(&back, "Nested")
        }
    );
}
