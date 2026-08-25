//! The degrade ladder, and what a frame is named under (K-490,
//! docs/impl/ocio.md §3.3, §5.5).
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

/// The same, with one space asking for a transform v1 does not implement. It
/// parses; it is simply not usable, and must say which transform it wanted.
const REFUSED: &str = r#"
ocio_profile_version: 2
roles:
  scene_linear: lin
colorspaces:
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: fancy
    to_scene_reference: !<FixedFunctionTransform> {style: ACES_RedMod03}
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

    let (spaces, displays) = loaded.vocabulary();
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
fn a_config_using_something_lumit_does_not_implement_refuses_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, REFUSED).unwrap();

    let mut state = ColourState::default();
    state.sync(&doc_naming(&path));
    let loaded = state.loaded().unwrap();
    assert!(!loaded.usable());
    let why = loaded.problem.clone().expect("it says why");
    assert!(
        why.contains("FixedFunctionTransform"),
        "a refusal names the transform it refused: {why}"
    );
    // The transform is named, which is the promise. Whether the colour space
    // is named too depends on where the refusal happened: a grammar this crate
    // does not read refuses at parse, before any space is being resolved; one
    // that parses and then cannot be walked refuses per space, and `load`
    // appends the name. Both are actionable; only the first is guaranteed.
    assert!(!why.is_empty());
}

#[test]
fn a_project_naming_no_config_is_exactly_what_it_was_before() {
    let mut state = ColourState::default();
    state.sync(&Document::new());
    assert!(state.loaded().is_none());
    assert_eq!(
        state.frame_identity(),
        0,
        "no config must name frames exactly as a build without OCIO did"
    );
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

/// **The K-490 asymmetry, in one test.** The preview of a project whose config
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

    // With no config at all, K-479's refusal stands untouched.
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

/// The built-in colour family is untouched by any of this: a project with no
/// config exports exactly as it did.
#[test]
fn the_built_in_colour_family_still_checks_without_a_config() {
    use lumit_render::export::ExportSpec;
    let spec = ExportSpec::default();
    spec.check_with_colour(&ColourState::default())
        .expect("the built-in family needs no config");
}

/// **The input transform reaches the decode pass** — the half of §5.2 that is a
/// decision rather than a shader. A footage layer's pixels carry the item's
/// colour space with them, because by the time the realiser sees a draw the
/// list has been flattened and there is no comp left to ask.
#[test]
fn a_footage_layers_pixels_carry_the_items_colour_space() {
    use lumit_core::model::LayerKind;
    use lumit_render::colour::footage_colour_space;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, GOOD).unwrap();
    let doc = doc_naming(&path);
    let item = doc.items[0].id();

    let footage = LayerKind::Footage { item };
    assert_eq!(
        footage_colour_space(&doc, &footage).as_deref(),
        Some("srgb_texture")
    );

    // Anything that is not footage has no interpretation to state.
    assert_eq!(
        footage_colour_space(
            &doc,
            &LayerKind::Solid {
                def: Uuid::now_v7()
            }
        ),
        None
    );

    // Nor does a footage item nobody has assigned.
    let mut plain = doc.clone();
    if let Some(ProjectItem::Footage(f)) = plain.items.first_mut() {
        f.colour_space = None;
    }
    assert_eq!(footage_colour_space(&plain, &footage), None);
}

/// One table per distinct space the project's footage names, and none at all
/// when there is nothing to do — which is what keeps an ordinary project's
/// render exactly as it was.
#[test]
fn one_input_table_is_uploaded_per_distinct_colour_space() {
    use lumit_render::colour::InputTransforms;

    let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
        eprintln!("no adapter here");
        return;
    };
    let engine = lumit_gpu::ColourEngine::new(&ctx);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, GOOD).unwrap();
    let mut doc = doc_naming(&path);
    // A second item on the same space, and a third on a name the config does
    // not have. Two items, one table; the unknown name gets none and falls back
    // to the built-in interpretation rather than to a wrong one.
    for (name, space) in [("b.mov", "srgb_texture"), ("c.mov", "no_such_space")] {
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: Uuid::now_v7(),
            name: name.into(),
            media: MediaRef {
                relative_path: name.into(),
                absolute_path: name.into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            colour_space: Some(space.into()),
            extra: serde_json::Map::new(),
        }));
    }

    let mut state = ColourState::default();
    state.sync(&doc);
    let built = InputTransforms::build(&doc, &state, &ctx, &engine);
    assert!(built.get("srgb_texture").is_some());
    assert!(built.get("no_such_space").is_none());
    assert!(!built.is_empty());

    // A project with no config uploads nothing at all.
    let none = ColourState::default();
    assert!(InputTransforms::build(&doc, &none, &ctx, &engine).is_empty());

    // And neither does one whose config went missing — the preview degrades to
    // the built-in interpretation rather than to no picture.
    std::fs::remove_file(&path).unwrap();
    state.sync(&doc);
    assert!(InputTransforms::build(&doc, &state, &ctx, &engine).is_empty());
}
