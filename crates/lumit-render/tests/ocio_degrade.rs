//! The degrade ladder, and what a frame is named under
//! (docs/impl/ocio.md §3.3, §5.5).
//!
//! **In plain terms.** A colour config is a file on disk, and files move, get
//! deleted, and get written by people using features Lumit has not implemented.
//! None of that may stop the project opening or the picture appearing. But it
//! must stop an *export*, because a wrong colour space in a file somebody hands
//! over is worse than an export that did not run. This file checks both halves
//! of that, and checks that changing any of it renames the frames it affects,
//! so nothing stale is ever handed back.
//!
//! No graphics card involved: everything here is the decision-making, not the
//! drawing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{ColourManagement, Document, FootageItem, MediaRef, ProjectItem};
use lumit_render::colour::{ColourState, Edge};
use uuid::Uuid;

/// A small, complete config: one space that is not the reference, one display
/// with one view, and the roles the bridge reads.
const GOOD: &str = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin
  reference: ref
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
colorspaces:
  - !<ColorSpace>
    name: ref
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: srgb_texture
    to_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0]}
  - !<ColorSpace>
    name: out_srgb
    from_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
"#;

fn doc_naming(path: &std::path::Path) -> Document {
    let mut doc = Document::new();
    doc.colour = ColourManagement {
        config: Some(MediaRef {
            relative_path: "config.ocio".into(),
            absolute_path: path.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        }),
        working_space: Default::default(),
    };
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: None,
        id: Uuid::now_v7(),
        name: "shot.mov".into(),
        media: MediaRef {
            relative_path: "shot.mov".into(),
            absolute_path: "shot.mov".into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: Some("srgb_texture".into()),
        extra: serde_json::Map::new(),
    }));
    doc
}

#[test]
fn a_config_that_loads_is_usable_and_offers_its_vocabulary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, GOOD).unwrap();

    let mut state = ColourState::default();
    state.sync(&doc_naming(&path));
    let loaded = state.loaded().expect("a config is named");
    assert!(loaded.usable(), "{:?}", loaded.problem);
    assert!(loaded.problem.is_none());

    let (spaces, displays, looks) = loaded.vocabulary();
    assert!(looks.is_empty(), "{looks:?}");
    assert!(spaces.iter().any(|s| s == "srgb_texture"), "{spaces:?}");
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].0, "sRGB");
    assert_eq!(displays[0].1, vec!["Standard".to_string()]);

    // And all three edges bake.
    assert!(loaded
        .artefact(&Edge::Input("srgb_texture".into()))
        .is_some());
    assert!(loaded
        .artefact(&Edge::DisplayView("sRGB".into(), "Standard".into()))
        .is_some());
    assert!(loaded.artefact(&Edge::Output("out_srgb".into())).is_some());
    // A name the config does not have is simply not there — not a panic, and
    // not a wrong table.
    assert!(loaded.artefact(&Edge::Input("nonsense".into())).is_none());
}

#[test]
fn a_config_that_vanished_degrades_calmly_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, GOOD).unwrap();
    let doc = doc_naming(&path);

    let mut state = ColourState::default();
    state.sync(&doc);
    assert!(state.loaded().unwrap().usable());
    let named_with = state.frame_identity();

    std::fs::remove_file(&path).unwrap();
    state.sync(&doc);
    let loaded = state.loaded().expect("the project still names a config");
    assert!(!loaded.usable(), "a missing file cannot be usable");
    let why = loaded.problem.clone().expect("and it says why");
    assert!(
        why.contains("could not be read") && why.contains("built-in"),
        "the sentence must name the fault and the fallback: {why}"
    );
    // Rendering falls back to the built-in family, which is what "no config"
    // means for a frame's name.
    assert_eq!(state.frame_identity(), 0);
    assert_ne!(named_with, 0, "a usable config does name frames");
    // Every edge is refused rather than approximated.
    assert!(loaded
        .artefact(&Edge::Input("srgb_texture".into()))
        .is_none());
}

#[test]
fn editing_the_config_on_disk_gives_every_frame_a_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, GOOD).unwrap();
    let doc = doc_naming(&path);

    let mut state = ColourState::default();
    state.sync(&doc);
    let before = state.frame_identity();

    // The same config with one number changed: different bytes, different
    // pictures, so different names (docs/impl/ocio.md §5.5).
    std::fs::write(&path, GOOD.replace("2.4, 2.4, 2.4", "2.2, 2.2, 2.2")).unwrap();
    state.sync(&doc);
    let after = state.frame_identity();
    assert_ne!(before, after, "an edited config must retire its frames");
    assert!(state.loaded().unwrap().usable());

    // And syncing again with nothing changed keeps the name, so a project that
    // is merely being redrawn does not throw its cache away every frame.
    state.sync(&doc);
    assert_eq!(state.frame_identity(), after);
}

/// **The asymmetry, in one test.** The preview of a project whose config
/// is missing still renders — the state above says so — but the export of a
/// file into one of that config's colour spaces refuses, and the refusal says
/// what went wrong rather than "not available in this build".
#[test]
fn preview_degrades_where_delivery_refuses() {
    use lumit_render::export::{ColourSpace, ExportSpec};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, GOOD).unwrap();
    let doc = doc_naming(&path);

    let spec = ExportSpec {
        colour_space: ColourSpace::Ocio("out_srgb".into()),
        ..Default::default()
    };

    // With no config at all, the refusal stands untouched.
    let none = ColourState::default();
    assert!(spec.check_with_colour(&none).is_err());

    // With the config loaded, the name is honoured.
    let mut state = ColourState::default();
    state.sync(&doc);
    spec.check_with_colour(&state)
        .expect("a name the loaded config has must be deliverable");

    // A name the config does not have still refuses, and names itself.
    let wrong = ExportSpec {
        colour_space: ColourSpace::Ocio("no_such_space".into()),
        ..Default::default()
    };
    let err = wrong.check_with_colour(&state).expect_err("refuses");
    assert!(err.contains("no_such_space"), "{err}");

    // And the moment the config goes, the export that worked a second ago
    // refuses — with the missing-file sentence, not a generic one.
    std::fs::remove_file(&path).unwrap();
    state.sync(&doc);
    let err = spec.check_with_colour(&state).expect_err("refuses");
    assert!(
        err.contains("out_srgb") && err.contains("could not be read"),
        "{err}"
    );
}

/// An ACES-shaped config: AP0 reference with the interchange role, ACEScg as
/// `scene_linear`, and an sRGB view, for the working-space tests.
fn aces_shaped() -> String {
    use lumit_colour::matrix;
    let m = matrix::rgb_to_rgb(&matrix::AP1, &matrix::AP0).unwrap();
    let row = |r: usize| format!("{}, {}, {}, 0", m[r * 4], m[r * 4 + 1], m[r * 4 + 2]);
    format!(
        r#"
ocio_profile_version: 2
roles:
  aces_interchange: ap0
  scene_linear: acescg
displays:
  sRGB:
    - !<View> {{name: Standard, colorspace: out_srgb}}
colorspaces:
  - !<ColorSpace>
    name: ap0
  - !<ColorSpace>
    name: acescg
    to_scene_reference: !<MatrixTransform> {{matrix: [{}, {}, {}, 0, 0, 0, 1]}}
  - !<ColorSpace>
    name: out_srgb
    from_scene_reference: !<ExponentWithLinearTransform> {{gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}}
"#,
        row(0),
        row(1),
        row(2)
    )
}

/// The config-defined working space (docs/impl/ocio.md §2.1): the project
/// composites in `scene_linear`, untagged footage and the built-in view are
/// carried across by the interchange matrix, and the choice renames frames.
#[test]
fn a_config_defined_working_space_carries_the_built_in_edges_across() {
    use lumit_core::model::WorkingSpace;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, aces_shaped()).unwrap();
    let mut doc = doc_naming(&path);

    let mut state = ColourState::default();
    state.sync(&doc);
    let ours = state.loaded().expect("named");
    assert!(ours.usable(), "{:?}", ours.problem);
    assert_eq!(ours.working_space(), None);
    assert!(ours.rec709_to_working().is_none());
    assert!(
        ours.artefact(&Edge::Untagged).is_none(),
        "Rec.709 working: the hardware decode"
    );
    assert!(ours.artefact(&Edge::BuiltinDisplay).is_none());
    let named_ours = state.frame_identity();

    doc.colour.working_space = WorkingSpace::ConfigSceneLinear;
    state.sync(&doc);
    let theirs = state.loaded().expect("named");
    assert!(theirs.usable(), "{:?}", theirs.problem);
    assert_eq!(theirs.working_space(), Some("acescg"));
    assert!(theirs.rec709_to_working().is_some());
    assert_ne!(
        state.frame_identity(),
        named_ours,
        "the choice renames every frame"
    );

    // Untagged footage: sRGB decode then Rec.709 → ACEScg. A neutral stays
    // neutral (the interchange matrix is white-adapted) and a Rec.709 red
    // does not.
    let untagged = theirs.artefact(&Edge::Untagged).expect("bakes");
    let grey = untagged.eval([0.5; 3]);
    assert!((grey[0] - 0.214).abs() < 2e-3, "{grey:?}");
    assert!((grey[0] - grey[1]).abs() < 1e-3 && (grey[1] - grey[2]).abs() < 1e-3);
    let red = untagged.eval([1.0, 0.0, 0.0]);
    assert!(
        red[1] > 0.01 && red[2] > 0.0,
        "Rec.709 red has green and blue in AP1: {red:?}"
    );

    // The built-in view undoes it: the same bytes come back.
    let shown = theirs.artefact(&Edge::BuiltinDisplay).expect("bakes");
    for c in [[0.5f32; 3], [1.0, 0.0, 0.0], [0.2, 0.7, 0.9]] {
        let back = shown.eval(untagged.eval(c));
        for k in 0..3 {
            assert!((back[k] - c[k]).abs() < 4e-3, "{c:?} came back as {back:?}");
        }
    }

    // A config that cannot place its scene_linear takes it as Rec.709: the
    // built-in edges stay the hardware ones, as compose-through always was.
    std::fs::write(&path, GOOD).unwrap();
    state.sync(&doc);
    let legacy = state.loaded().expect("named");
    assert!(legacy.usable(), "{:?}", legacy.problem);
    assert_eq!(legacy.working_space(), Some("lin"));
    assert!(legacy.rec709_to_working().is_none());
    assert!(legacy.artefact(&Edge::Untagged).is_none());
}
