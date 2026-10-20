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
use lumit_core::model::{
    BlendMode, Composition, Document, EffectNamespace, EffectValue, Layer, LayerKind, MatteChannel,
    ProjectItem,
};
use lumit_core::time::Rational;
use lumit_import::{map_capture, ImportReport, Reason};

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

/// **A layer's audio level comes across, and an unbalanced pair says so.**
///
/// After Effects gives a layer one level per channel and Lumit gives it one,
/// so a mix that rides the two apart cannot arrive whole — the left channel
/// does, and the row carries what the right one was. What this really guards
/// is the flat case underneath it: a level read as 0 dB plays a song mixed
/// twenty decibels down at full.
#[test]
fn a_layers_audio_level_comes_across_and_an_unbalanced_pair_is_reported() {
    let (doc, report) = mapped("synthetic.lum-bundle");
    let clip = layer(comp(&doc, "Main"), "clip.mp4");
    assert!(
        (clip.volume_db.value_at(0.0) - -6.0).abs() < 1e-9,
        "the left channel's level, not a flat nought"
    );
    assert!(reported(&report, |r| matches!(
        r,
        Reason::AudioLevelsDiffer { left, right }
            if (left - -6.0).abs() < 1e-9 && (right - -12.0).abs() < 1e-9
    )));

    // A layer After Effects said nothing about stays at unity.
    let solid = layer(comp(&doc, "Main"), "Black Solid 1");
    assert_eq!(solid.volume_db.value_at(0.0), 0.0);
}

/// **A placeholder's parameters are real Lumit properties: they animate, and
/// what has no property shape is kept verbatim.**
///
/// docs/11 §6 in full, on a third-party effect that will never be in the table:
/// the keyframed Speed becomes a keyframed property, the colour becomes a
/// colour, the unreadable blob and the layer reference are kept in the `ae`
/// namespace, and the whole thing is switched off exactly as it was.
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

/// **An After Effects image sequence becomes a Lumit image sequence**.
///
/// The mapping is small but load-bearing in two directions. It has to carry
/// the fact through — a run of stills that imports as a single still shows one
/// frame for a shot that is a thousand — and it has to carry the *folder*, not
/// a file, because the folder is all the .aep names. Turning that folder into
/// the run's first frame is resolution's job, and it happens on open.
#[test]
fn an_after_effects_image_sequence_maps_to_a_sequence_item() {
    let mut capture = lumit_import::Capture::default();
    capture.items.push(lumit_import::capture::Item {
        id: Some(1),
        name: Some("Depth".into()),
        kind: Some("footage".into()),
        path: Some("/media/Cine3/Depth".into()),
        is_sequence: Some(true),
        sequence_prefix: Some("Depth".into()),
        sequence_suffix: Some("_depth.exr".into()),
        ..Default::default()
    });
    capture.items.push(lumit_import::capture::Item {
        id: Some(2),
        name: Some("World.avi".into()),
        kind: Some("footage".into()),
        path: Some("/media/Cine3/World.avi".into()),
        ..Default::default()
    });

    let (doc, _) = lumit_import::map_capture(&capture);
    let footage = |name: &str| match doc.item(item_id(&doc, name)) {
        Some(ProjectItem::Footage(f)) => f.clone(),
        other => panic!("expected footage, got {other:?}"),
    };

    let run = footage("Depth");
    assert_eq!(
        run.sequence_fps(),
        Some((25, 1)),
        "stills carry no rate, and the .aep's conform rate is a preference \
         rather than a fact in the file — so it is the default"
    );
    assert_eq!(
        run.media.relative_path, "/media/Cine3/Depth",
        "the folder the run lives in, which is what the alias names"
    );
    let ae = run.extra.get("ae").expect("an ae namespace");
    assert_eq!(ae.get("sequence_prefix"), Some(&serde_json::json!("Depth")));
    assert_eq!(
        ae.get("sequence_suffix"),
        Some(&serde_json::json!("_depth.exr"))
    );

    assert_eq!(
        footage("World.avi").sequence,
        None,
        "a clip in the same folder is still one file"
    );
}

/// **A tracked camera arrives with its motion** (docs/11 Â§3).
///
/// The camera every tracker writes â€” HLAE, SynthEyes, a 3D application's
/// exporter â€” keys three things and only three: Position, Orientation and
/// Zoom. The whole lot used to be thrown away wholesale whenever the exporter
/// had also hung an expression on the property, which is how a time-remapped
/// camera is written.
///
/// So this is the shape of a real tracked camera, keyframe for keyframe: the
/// orientation on its own lanes, turning between keys as After Effects turns
/// it, the zoom out of the options group, and the After Effects expression
/// switched off with its text kept, so what drives the camera is the motion
/// that was tracked.
#[test]
fn a_tracked_cameras_keyframes_arrive_on_the_orientation_lanes_and_the_zoom() {
    let (doc, report) = map_capture(&camera_capture());
    let camera = layer(comp(&doc, "Track"), "Camera");

    // Position, three axes off one After Effects property.
    assert_eq!(keys(&camera.transform.position_x).len(), 3);
    assert_eq!(keys(&camera.transform.position_z).len(), 3);
    assert_eq!(keys(&camera.transform.position_x)[1].value, 20.0);
    assert_eq!(keys(&camera.transform.position_z)[2].value, 300.0);

    // Orientation, on its own lanes, angle for angle, and the turn matches at
    // every key.
    let orientation = [[10.0, 40.0, 1.0], [20.0, 45.0, 2.0], [30.0, 50.0, 3.0]];
    let [ox, oy, oz] = camera.transform.orientation.each_ref().map(keys);
    for (n, o) in orientation.into_iter().enumerate() {
        assert_eq!([ox[n].value, oy[n].value, oz[n].value], o, "key {n}");
        let turn = lumit_turn(camera, ox[n].time.to_f64());
        assert!(same_turn(turn, ae_turn(o, [0.0; 3])), "key {n}");
    }
    assert_eq!(ox[0].interp_out, SideInterp::Linear);
    assert!(is_still_zero(&camera.transform.rotation_x));
    // Between keys it takes the one shortest turn, as After Effects does,
    // halfway round at halfway.
    let at = |t: f64| camera.transform.orientation_at(t, &|p| p.value_at(t));
    let between = lumit_core::camera::turn_between_deg;
    let (a, mid, b) = (at(0.0), at(0.5), at(1.0));
    assert!((between(a, mid) - between(a, b) / 2.0).abs() < 1e-6);
    assert!((between(mid, b) - between(a, b) / 2.0).abs() < 1e-6);
    // The middle key is held in After Effects, and held here.
    assert_eq!(oy[1].interp_out, SideInterp::Hold);
    assert_eq!(at(1.5), at(1.0));

    // Zoom, out of the options group rather than the transform.
    let LayerKind::Camera { zoom, .. } = &camera.kind else {
        panic!("a camera");
    };
    let zoom = keys(zoom);
    assert_eq!(zoom.len(), 3);
    assert_eq!(zoom[0].value, 1000.0);
    assert_eq!(zoom[2].value, 1400.0);

    // The exporter's expression sits over those keys, and the keys are the
    // shot, so it drives nothing and is named in the report with its text.
    assert!(reported(&report, |r| matches!(
        r,
        Reason::ExpressionOverKeys { source } if source.contains("thisComp")
    )));
    assert_eq!(
        camera
            .transform
            .position_x
            .extra
            .get("ae")
            .and_then(|ae| ae.get("expression"))
            .and_then(serde_json::Value::as_str)
            .map(|e| e.contains("thisComp")),
        Some(true)
    );
}

/// **A 3D layer turned on both X and Y faces where After Effects faces it.**
///
/// After Effects turns by `Rx · Ry · Rz`, with Orientation outside that, and
/// Lumit by `Ry · Rx · Rz`. An imported layer turns in After Effects' order,
/// so its angles come across as they are. The matrix is the check, X at a
/// quarter turn included, where Lumit's own Y and Z would be one turn.
#[test]
fn a_3d_layer_turned_on_x_and_y_faces_where_after_effects_faces_it() {
    let turned = |index: u32, orientation: [f64; 3], [x, y, z]: [f64; 3]| {
        let float = |name: &str, v: f64| serde_json::json!({ "match_name": name, "value_type": "float", "value": v });
        serde_json::json!({
            "index": index,
            "name": format!("Turned {index}"),
            "kind": "null",
            "in_point": 0.0,
            "out_point": 4.0,
            "switches": { "three_d": true },
            "properties": [{
                "match_name": "ADBE Transform Group",
                "group": [
                    { "match_name": "ADBE Anchor Point", "value_type": "point3", "value": [5.0, 6.0, 7.0] },
                    { "match_name": "ADBE Scale", "value_type": "point3", "value": [100.0, 100.0, 50.0] },
                    { "match_name": "ADBE Orientation", "value_type": "point3", "value": orientation },
                    float("ADBE Rotate X", x),
                    float("ADBE Rotate Y", y),
                    float("ADBE Rotate Z", z)
                ]
            }]
        })
    };
    let cases = [
        ([0.0, 0.0, 0.0], [30.0, 45.0, 0.0]),
        ([0.0, 0.0, 0.0], [90.0, 45.0, 30.0]),
        ([10.0, 0.0, 20.0], [30.0, 45.0, -15.0]),
        // Straight down, where Lumit's Y and Z name the same turn.
        ([90.0, 0.0, 0.0], [0.0, 0.0, 30.0]),
    ];
    let layers: Vec<_> = (1..)
        .zip(cases)
        .map(|(i, (o, r))| turned(i, o, r))
        .collect();
    let capture: lumit_import::capture::Capture = serde_json::from_value(serde_json::json!({
        "items": [{ "id": 1, "kind": "comp", "name": "Turns" }],
        "comps": [{ "id": 1, "width": 1920, "height": 1080, "fps": 25.0, "duration": 4.0, "layers": layers }]
    }))
    .expect("the capture parses");

    let (doc, _) = map_capture(&capture);
    for (i, (o, r)) in (1..).zip(cases) {
        let turned = layer(comp(&doc, "Turns"), &format!("Turned {i}"));
        assert!(
            same_turn(lumit_turn(turned, 0.0), ae_turn(o, r)),
            "orientation {o:?}, rotation {r:?}"
        );
        // Orientation, the rotations, Anchor Z and Scale Z arrive as they are.
        let tr = &turned.transform;
        assert!(tr.after_effects_order);
        assert_eq!(tr.orientation.each_ref().map(|p| p.value_at(0.0)), o);
        let rows = [&tr.rotation_x, &tr.rotation_y, &tr.rotation].map(|p| p.value_at(0.0));
        assert_eq!(rows, r);
        assert_eq!(
            (tr.anchor_z.value_at(0.0), tr.scale_z.value_at(0.0)),
            (7.0, 50.0)
        );
    }
}

type Mat3 = lumit_core::camera::Mat3;

/// After Effects' turn for an Orientation and an X/Y/Z Rotation.
fn ae_turn(orientation: [f64; 3], rotation: [f64; 3]) -> Mat3 {
    use lumit_core::camera::rotation as lumit;
    let mul = |a: Mat3, b: Mat3| {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
            }
        }
        out
    };
    let xyz = |[x, y, z]: [f64; 3]| {
        mul(
            mul(lumit((x, 0.0, 0.0)), lumit((0.0, y, 0.0))),
            lumit((0.0, 0.0, z)),
        )
    };
    mul(xyz(orientation), xyz(rotation))
}

/// The turn a layer's Orientation and rotation lanes make at `t`.
fn lumit_turn(layer: &Layer, t: f64) -> Mat3 {
    let tr = &layer.transform;
    let orientation = tr.orientation_at(t, &|p| p.value_at(t));
    lumit_core::camera::rotation(lumit_core::camera::oriented(
        orientation,
        (
            tr.rotation_x.value_at(t),
            tr.rotation_y.value_at(t),
            tr.rotation.value_at(t),
        ),
        tr.after_effects_order,
    ))
}

fn is_still_zero(p: &lumit_core::anim::Property) -> bool {
    p.animation == lumit_core::anim::Animation::Static(0.0)
}

fn same_turn(a: Mat3, b: Mat3) -> bool {
    let mut cells = a.iter().flatten().zip(b.iter().flatten());
    cells.all(|(a, b)| (a - b).abs() < 1e-9)
}

/// A tracked camera as an exporter writes one: keyed Position, Orientation and
/// Zoom, each carrying the same After Effects expression.
fn camera_capture() -> lumit_import::capture::Capture {
    const EXPRESSION: &str = r#"valueAtTime(thisComp.layer("Remap").timeRemap)"#;
    let key = |t: f64, v: serde_json::Value, out: &str| serde_json::json!({ "t": t, "v": v, "in_interp": "LINEAR", "out_interp": out });
    serde_json::from_value(serde_json::json!({
        "items": [{ "id": 1, "kind": "comp", "name": "Track" }],
        "comps": [{
            "id": 1,
            "width": 1920,
            "height": 1080,
            "fps": 25.0,
            "duration": 4.0,
            "layers": [{
                "index": 1,
                "name": "Camera",
                "kind": "camera",
                "in_point": 0.0,
                "out_point": 4.0,
                "auto_orient": "NO_AUTO_ORIENT",
                "properties": [
                    {
                        "match_name": "ADBE Transform Group",
                        "group": [
                            {
                                "match_name": "ADBE Position",
                                "value_type": "point3",
                                "expression": EXPRESSION,
                                "expression_enabled": true,
                                "keyframes": [
                                    key(0.0, serde_json::json!([10.0, 100.0, 200.0]), "LINEAR"),
                                    key(1.0, serde_json::json!([20.0, 110.0, 250.0]), "LINEAR"),
                                    key(2.0, serde_json::json!([30.0, 120.0, 300.0]), "LINEAR")
                                ]
                            },
                            {
                                "match_name": "ADBE Orientation",
                                "value_type": "point3",
                                "expression": EXPRESSION,
                                "expression_enabled": true,
                                "keyframes": [
                                    key(0.0, serde_json::json!([10.0, 40.0, 1.0]), "LINEAR"),
                                    key(1.0, serde_json::json!([20.0, 45.0, 2.0]), "HOLD"),
                                    key(2.0, serde_json::json!([30.0, 50.0, 3.0]), "LINEAR")
                                ]
                            }
                        ]
                    },
                    {
                        "match_name": "ADBE Camera Options Group",
                        "group": [{
                            "match_name": "ADBE Camera Zoom",
                            "value_type": "float",
                            "expression": EXPRESSION,
                            "expression_enabled": true,
                            "keyframes": [
                                key(0.0, serde_json::json!(1000.0), "LINEAR"),
                                key(1.0, serde_json::json!(1200.0), "LINEAR"),
                                key(2.0, serde_json::json!(1400.0), "LINEAR")
                            ]
                        }]
                    }
                ]
            }]
        }]
    }))
    .expect("the capture parses")
}

/// **An effect is measured against the layer, not against the composition**.
///
/// After Effects runs an effect on the layer's own raster, so Motion Tile's
/// four per cents are per cents of *that* frame and its Tile Center is a point
/// in it. The capture below is the shape the owner's project has: a 2560 × 1088
/// precomp placed in a 1920 × 816 comp, with a Motion Tile whose only touched
/// control is Output Width — everything else left at After Effects' default,
/// and therefore absent from the capture entirely. Read against the comp, the
/// defaults landed at the comp's middle and the comp's width, so the tile was
/// cut from up and to the left of the layer's centre and the window with it;
/// that is the offset the effect showed. Read against the layer, every number
/// is the one After Effects means.
#[test]
fn a_motion_tile_is_measured_against_its_own_layer_not_the_composition() {
    let capture: lumit_import::capture::Capture = serde_json::from_value(serde_json::json!({
        "items": [
            { "id": 1, "kind": "comp", "name": "Clips" },
            { "id": 2, "kind": "comp", "name": "Border" }
        ],
        "comps": [
            { "id": 2, "width": 2560, "height": 1088, "fps": 25.0, "duration": 4.0, "layers": [] },
            {
                "id": 1,
                "width": 1920,
                "height": 816,
                "fps": 25.0,
                "duration": 4.0,
                "layers": [{
                    "index": 1,
                    "name": "Border",
                    "kind": "precomp",
                    "source_id": 2,
                    "in_point": 0.0,
                    "out_point": 4.0,
                    "properties": [{
                        "match_name": "ADBE Effect Parade",
                        "group": [{
                            "match_name": "ADBE Tile",
                            "name": "Motion Tile",
                            "group": [{
                                "match_name": "ADBE Tile-0004",
                                "name": "Output Width",
                                "value_type": "float",
                                "value": 125.0
                            }]
                        }]
                    }]
                }]
            }
        ]
    }))
    .expect("the capture parses");

    let (doc, _report) = map_capture(&capture);
    let tile = &layer(comp(&doc, "Clips"), "Border").effects[0];
    assert_eq!(tile.effect.match_name, "tile");
    let at = |id: &str| tile.float_at(id, 0.0).expect("a float parameter");
    // The layer's own middle, and the layer's own width and height.
    assert_eq!((at("tile_centre_x"), at("tile_centre_y")), (1280.0, 544.0));
    assert_eq!((at("tile_width"), at("tile_height")), (2560.0, 1088.0));
    // 125 % of the layer is 3200, not 125 % of the comp's 1920.
    assert_eq!(at("output_width"), 3200.0);
    // And the untouched Output Height is the layer's own, whole.
    assert_eq!(at("output_height"), 1088.0);
}

/// **An ambient light arrives as one, its keys with it.**
///
/// Light options used to come across as their still value alone, and an
/// ambient light as a point. Its intensity's keys arrive as Lumit's own
/// percentages, and its colour's in scene-linear light.
#[test]
fn an_ambient_light_keeps_its_kind_and_its_keyframes() {
    let (doc, report) = mapped("edges.lum-bundle");
    let LayerKind::Light { light } = &layer(comp(&doc, "Edges"), "Key light").kind else {
        panic!("a light");
    };
    assert_eq!(light.kind, lumit_core::model::LightKind::Ambient);
    assert_eq!(
        comp(&doc, "Edges").lighting,
        lumit_core::model::CompLighting::AfterEffects { linear: false },
        "lit as After Effects lights it"
    );
    assert_eq!(keys(&light.intensity).len(), 2);
    assert_eq!(light.intensity.value_at(2.0), 150.0);
    let green = keys(&light.colour[1]);
    assert_eq!(green.len(), 2);
    assert!(
        (green[1].value - 0.214).abs() < 1e-3,
        "half grey, made linear"
    );
    assert!(!reported(&report, |r| matches!(
        r,
        Reason::LightKindApproximated { .. }
    )));
}
