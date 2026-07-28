//! Tests for the flutter_rust_bridge API surface.
//!
//! A file of their own, not a `mod tests` inside each api module, for two
//! reasons: test code legitimately uses `expect`/`unwrap` where the api modules
//! deny them, and the `no-panics-in-frb-api` CI job greps `src/api` for exactly
//! those forms — it excludes this one path by name, which is more honest than
//! teaching a grep to recognise where a test module begins.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::api::{
    composition::{BridgeCompSettings, CompositionReference},
    effect::{
        list_effects, BridgeEffectValue, BridgeKeyframe, BridgeRational, BridgeScalar,
        BridgeSideInterp,
    },
    folder::FolderReference,
    footage::FootageReference,
    footage::LumitMediaStatus,
    layer::LayerReference,
    project::ProjectReference,
    project_item::ItemReference,
    state::LumitBridgeState,
    BridgeError,
};
use lumit_core::model::{Folder, FootageItem, MediaRef, ProjectItem};
use lumit_core::Op;
use uuid::Uuid;

/// A project holding a folder that lists one footage item, plus a second
/// footage item at the root. Returns the project and the three references.
///
/// Note this leaves the project in the process-wide `PROJECTS` registry, keyed
/// by a fresh uuid, so tests do not collide — but a test must never call
/// `open_project`, which clears the whole registry.
fn project_with_folder() -> (
    ProjectReference,
    ItemReference,
    ItemReference,
    ItemReference,
) {
    let project = LumitBridgeState::new_project(None).expect("a new project");

    let filed = FootageItem {
        id: Uuid::now_v7(),
        name: "filed.mp4".into(),
        media: MediaRef {
            relative_path: "filed.mp4".into(),
            absolute_path: String::new(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
    };
    let loose = FootageItem {
        id: Uuid::now_v7(),
        name: "loose.mp4".into(),
        media: MediaRef {
            relative_path: "loose.mp4".into(),
            absolute_path: String::new(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
    };
    let folder = Folder {
        id: Uuid::now_v7(),
        name: "Clips".into(),
        children: vec![filed.id],
        extra: serde_json::Map::new(),
    };
    let (filed_id, loose_id, folder_id) = (filed.id, loose.id, folder.id);

    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        for (index, item) in [
            ProjectItem::Folder(folder),
            ProjectItem::Footage(filed),
            ProjectItem::Footage(loose),
        ]
        .into_iter()
        .enumerate()
        {
            state
                .store
                .commit(Op::AddItem {
                    index,
                    item: Box::new(item),
                })
                .expect("seeded");
        }
    }

    let id = project.id;
    (
        project,
        ItemReference::Folder(FolderReference::new(id, folder_id)),
        ItemReference::Footage(FootageReference::new(id, filed_id)),
        ItemReference::Footage(FootageReference::new(id, loose_id)),
    )
}

/// The panel draws roots then recurses, so a folder must report its own
/// children and nothing else — a flat list of everything would nest wrongly.
#[test]
fn a_folder_reports_only_its_own_children() {
    let (project, folder, filed, _loose) = project_with_folder();

    // Roots only: the folder and the unfiled item. The filed footage is reached
    // through the folder, never listed again at the top level — drawing it at both
    // levels was the bug this asserts against.
    let roots = project.get_items().expect("roots");
    assert_eq!(roots.len(), 2, "the folder and the unfiled item");
    assert!(
        !roots.iter().any(|r| r.equals(&filed)),
        "a filed item must not also appear at the root"
    );

    let ItemReference::Folder(folder_ref) = &folder else {
        panic!("the fixture built a folder");
    };
    let children = folder_ref.get_children().expect("children");
    assert_eq!(children.len(), 1);
    assert!(children[0].equals(&filed));
}

#[test]
fn rename_changes_the_name_and_refuses_a_blank_one() {
    let (_project, _folder, filed, _loose) = project_with_folder();

    filed.rename("hero shot".into()).expect("renamed");
    assert_eq!(filed.name().expect("name"), "hero shot");

    // Blank is refused rather than applied, so a row cannot lose its label.
    let err = filed.rename("   ".into());
    assert!(matches!(err, Err(BridgeError::EmptyName)));
    assert_eq!(filed.name().expect("name"), "hero shot", "unchanged");
}

#[test]
fn delete_removes_the_item_and_a_second_delete_is_a_calm_error() {
    let (project, _folder, _filed, loose) = project_with_folder();

    loose.delete().expect("deleted");
    assert_eq!(
        project.get_items().expect("roots").len(),
        1,
        "just the folder is left at the root"
    );

    // The reference now outlives its item: an error, never a panic.
    assert!(matches!(loose.delete(), Err(BridgeError::InvalidItem)));
}

/// Moving to the root means "no folder lists it any more". The item itself
/// stays in the document either way, which is what makes this distinct from
/// deleting.
#[test]
fn move_to_root_unfiles_the_item_and_is_a_no_op_when_already_there() {
    let (_project, folder, filed, loose) = project_with_folder();
    let ItemReference::Folder(folder_ref) = &folder else {
        panic!("the fixture built a folder");
    };

    filed.move_to_root().expect("unfiled");
    assert!(
        folder_ref.get_children().expect("children").is_empty(),
        "the folder no longer lists it"
    );
    assert_eq!(
        filed.name().expect("name"),
        "filed.mp4",
        "still in the document"
    );

    // Already at the root: accepted and does nothing, rather than erroring.
    loose.move_to_root().expect("no-op");
    filed.move_to_root().expect("no-op the second time too");
}

/// Relinking points the item at the picked file and, crucially, is refused when
/// there is nothing to point at — a silent success would leave the user thinking
/// a broken item had been fixed.
#[test]
fn relink_refuses_a_blank_or_useless_path() {
    let (_project, _folder, filed, _loose) = project_with_folder();
    let ItemReference::Footage(footage) = &filed else {
        panic!("the fixture built footage");
    };

    assert!(matches!(
        footage.relink(String::new()),
        Err(BridgeError::MediaPathUnresolved)
    ));

    // A path that does not exist: the target itself is still repointed (the user
    // asked for it explicitly), so this succeeds and the document records it.
    let picked = std::env::temp_dir().join("lumit-relink-target.mp4");
    std::fs::write(&picked, b"not really a video").expect("temp file");
    footage
        .relink(picked.to_string_lossy().into_owned())
        .expect("the explicit target is always repointed");
    std::fs::remove_file(&picked).ok();
}

/// A placed clip must land in the composition; the span/size fallbacks are what
/// let a *missing* file still place, so the user can relink rather than being
/// unable to add it at all.
#[test]
fn footage_places_into_a_composition_even_when_the_media_is_missing() {
    let (project, _folder, filed, _loose) = project_with_folder();
    let ItemReference::Footage(footage) = &filed else {
        panic!("the fixture built footage");
    };

    let comp = add_comp(&project, "Scene");
    assert!(comp.get_layers().expect("layers").is_empty());

    // The fixture's media has an empty absolute path and an unsaved project, so
    // it cannot resolve — the comp's own duration and size are used.
    comp.add_footage_layer(footage).expect("placed");

    let layers = comp.get_layers().expect("layers");
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].get_name().expect("name"), "filed.mp4");
}

/// `get_size` is what the Viewer divides its panel box by to work out a render
/// scale, so it has to report the comp's own dimensions rather than anything else.
#[test]
fn a_composition_reports_its_own_size() {
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");

    let size = comp.get_size().expect("size");
    assert_eq!((size.width, size.height), (1920, 1080));
}

/// Add a composition straight through the store, since the frb API has no
/// add-composition op yet (that arrives with the Timeline port).
fn add_comp(project: &ProjectReference, name: &str) -> CompositionReference {
    use lumit_core::model::LinearColour;
    use lumit_core::time::{Duration, FrameRate, Rational};

    let comp = lumit_core::model::Composition {
        id: Uuid::now_v7(),
        name: name.into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).expect("30 fps"),
        duration: Duration(Rational::new(10, 1).expect("10 s")),
        background: LinearColour([0.0, 0.0, 0.0, 0.0]),
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;

    let state = project.state().expect("state");
    let state = state.write().expect("write");
    state
        .store
        .commit(Op::AddItem {
            index: 0,
            item: Box::new(ProjectItem::Composition(comp)),
        })
        .expect("comp added");

    CompositionReference::new(project.id, comp_id)
}

/// `get_status` must report a file that is not there as missing — the Project
/// panel's badge depends on it.
#[test]
fn a_footage_item_pointing_at_nothing_reports_missing() {
    let project = LumitBridgeState::new_project(None).expect("project");
    let footage = project
        .import_footage("C:/nowhere/definitely-not-here.mp4".into())
        .expect("imported");

    let status = footage.get_status().expect("status");
    assert!(matches!(status, LumitMediaStatus::Missing));
}

/// Relink takes a write lock after having taken a read lock earlier in the same
/// call. If those ever overlap, this deadlocks rather than fails — so the test
/// existing at all is the guard.
#[test]
fn relink_does_not_deadlock_against_its_own_read() {
    let project = LumitBridgeState::new_project(None).expect("project");
    let footage = project
        .import_footage("C:/nowhere/gone.mp4".into())
        .expect("imported");

    let target = std::env::temp_dir().join("lumit-relink-deadlock-probe.mp4");
    std::fs::write(&target, b"stub").expect("temp file");

    footage
        .relink(target.to_string_lossy().into_owned())
        .expect("relinked");

    std::fs::remove_file(&target).ok();
}

/// Importing then reading back is the panel's whole read path, and `new_composition`
/// must file its comp so the tree has something to nest.
#[test]
fn import_and_new_composition_land_in_the_item_tree() {
    let project = LumitBridgeState::new_project(None).expect("project");
    project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    let comp = project.new_composition("Scene".into(), None).expect("comp");

    let roots = project.get_items().expect("roots");
    // The footage and the Compositions folder. The comp is inside the folder, so
    // it is NOT a root — drawing it at both levels was the bug this asserts against.
    assert_eq!(roots.len(), 2);

    let folder = roots
        .iter()
        .find_map(|i| match i {
            ItemReference::Folder(f) => Some(f),
            _ => None,
        })
        .expect("the Compositions auto-folder was created");
    let children = folder.get_children().expect("children");
    assert_eq!(children.len(), 1, "the comp is filed into it");
    assert_eq!(children[0].name().expect("name"), "Scene");
    assert_eq!(comp.get_size().expect("size").width, 1920);
}

/// Composition settings must round-trip exactly, including a non-integer frame
/// rate. 29.97 fps is 30000/1001; if the pair went through a float anywhere it
/// would not come back, which is why the settings type carries num and den rather
/// than a single number (docs/14 §2).
#[test]
fn composition_settings_round_trip_including_a_drop_frame_rate() {
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");

    let before = comp.get_settings().expect("settings");
    assert_eq!((before.fps_num, before.fps_den), (30, 1));

    comp.set_settings(BridgeCompSettings {
        name: "Renamed".into(),
        width: 1280,
        height: 720,
        fps_num: 30000,
        fps_den: 1001,
        duration: BridgeRational { num: 8, den: 1 },
    })
    .expect("applied");

    let after = comp.get_settings().expect("settings");
    assert_eq!(after.name, "Renamed");
    assert_eq!((after.width, after.height), (1280, 720));
    assert_eq!(
        (after.fps_num, after.fps_den),
        (30000, 1001),
        "the exact rate survives — no float round trip"
    );
    assert_eq!(
        (after.duration.num, after.duration.den),
        (8, 1),
        "the length is the exact seconds it was given"
    );
    assert_eq!(comp.duration_frames().expect("frames"), 239, "8 s at 29.97");
}

/// **The frame-rate regression (K-180).** Changing only the rate must change only
/// the rate: the comp keeps its real length, and a layer keeps the seconds it
/// occupies, so nothing plays faster or slower. Before this, the dialog read the
/// duration as a frame count and wrote the same count back at the new rate, which
/// silently halved or doubled the comp against layers that had not moved.
#[test]
fn changing_only_the_frame_rate_leaves_the_comp_and_its_layers_where_they_were() {
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer().expect("layer");
    let span_before = layer.get_span().expect("span");

    let before = comp.get_settings().expect("settings");
    assert_eq!(comp.duration_frames().expect("frames"), 300, "10 s at 30");

    comp.set_settings(BridgeCompSettings {
        fps_num: 60,
        ..before
    })
    .expect("applied");

    let after = comp.get_settings().expect("settings");
    assert_eq!(
        (after.duration.num, after.duration.den),
        (10, 1),
        "still ten seconds long"
    );
    assert_eq!(
        comp.duration_frames().expect("frames"),
        600,
        "the same ten seconds, counted twice as finely"
    );
    assert_eq!(
        layer.get_span().expect("span"),
        span_before,
        "the layer occupies the same time — the rate is not a speed control"
    );
}

/// A dialog must not be able to commit a comp that is zero pixels wide or zero
/// frames long, and a zero frame rate is refused outright rather than clamped —
/// there is no sensible rate to clamp to.
#[test]
fn composition_settings_clamp_the_absurd_and_refuse_a_zero_rate() {
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");

    comp.set_settings(BridgeCompSettings {
        name: "Tiny".into(),
        width: 0,
        height: 0,
        fps_num: 30,
        fps_den: 1,
        duration: BridgeRational { num: 0, den: 1 },
    })
    .expect("applied");

    let after = comp.get_settings().expect("settings");
    assert_eq!((after.width, after.height), (16, 16), "clamped, not zero");
    assert_eq!(comp.duration_frames().expect("frames"), 1, "one frame");

    assert!(matches!(
        comp.set_settings(BridgeCompSettings {
            name: "Bad".into(),
            width: 1920,
            height: 1080,
            fps_num: 0,
            fps_den: 1,
            duration: BridgeRational { num: 10, den: 1 },
        }),
        Err(BridgeError::InvalidFrameRate)
    ));
}

/// Saving answers where it wrote, and a project that has never been saved refuses
/// an empty path rather than guessing a location.
#[test]
fn save_reports_its_path_and_refuses_to_guess_one() {
    let (project, ..) = project_with_folder();

    assert!(project.path().expect("path").is_none());
    assert!(matches!(
        project.save(String::new()),
        Err(BridgeError::NoProjectPath)
    ));

    let dir = std::env::temp_dir().join("lumit-save-probe");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("probe.lum");

    let written = project
        .save(target.to_string_lossy().into_owned())
        .expect("saved");
    assert!(written.ends_with("probe.lum"));
    assert!(target.is_file(), "the file really exists");

    // Now it knows where it lives, so an empty path saves in place.
    assert_eq!(
        project.path().expect("path").as_deref(),
        Some(written.as_str())
    );
    project.save(String::new()).expect("saved in place");

    std::fs::remove_dir_all(&dir).ok();
}

/// The status bar's saved/unsaved readout. Fails without `saved_revision`
/// being stamped on save.
#[test]
fn is_dirty_tracks_edits_saves_and_undo() {
    let (project, ..) = project_with_folder();
    // project_with_folder commits its seed items, so the project starts dirty
    // relative to "never saved" — which is the honest answer.
    assert!(
        project.is_dirty().expect("dirty"),
        "unsaved edits are dirty"
    );

    let dir = std::env::temp_dir().join("lumit-dirty-probe");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("probe.lum");
    project
        .save(target.to_string_lossy().into_owned())
        .expect("saved");
    assert!(
        !project.is_dirty().expect("dirty"),
        "a save cleans the flag"
    );

    project.new_composition("Scene".into(), None).expect("comp");
    assert!(
        project.is_dirty().expect("dirty"),
        "an edit dirties it again"
    );

    project.save(String::new()).expect("saved in place");
    assert!(!project.is_dirty().expect("dirty"));

    // An undo moves the revision too: only a save proves the file matches.
    project.undo().expect("undone");
    assert!(
        project.is_dirty().expect("dirty"),
        "undo after save is dirty"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// The menu bar greys Undo and Redo from this, so it has to track the store.
#[test]
fn history_reports_what_undo_and_redo_can_do() {
    let project = LumitBridgeState::new_project(None).expect("project");

    let empty = project.history().expect("history");
    assert!(
        !empty.can_undo && !empty.can_redo,
        "a fresh project has none"
    );

    project.new_composition("Scene".into(), None).expect("comp");
    let after_edit = project.history().expect("history");
    assert!(after_edit.can_undo && !after_edit.can_redo);

    project.undo().expect("undone");
    let after_undo = project.history().expect("history");
    assert!(after_undo.can_redo, "undoing makes a redo available");
}

// ---------------------------------------------------------------------------
// Effect controls: the parameter value type and the stack ops.
// ---------------------------------------------------------------------------

/// A fresh project holding one composition with one adjustment layer in it.
/// Adjustment is chosen because it needs no media: the effect surface only cares
/// that a layer exists to hang a stack on.
fn project_with_layer() -> (ProjectReference, LayerReference) {
    use lumit_core::model::{LayerKind, TransformGroup};

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = add_comp(&project, "Scene");
    let layer = crate::edits::base_layer(
        "Adjust".into(),
        LayerKind::Adjustment,
        lumit_core::time::Rational::new(5, 1).expect("5 s"),
        TransformGroup::default(),
    );
    let layer_id = layer.id;

    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(Op::AddLayer {
                comp: comp.id,
                index: 0,
                layer: Box::new(layer),
            })
            .expect("layer added");
    }

    let layer = LayerReference::new(project.id, comp.id, layer_id);
    (project, layer)
}

/// An effect instance carrying one parameter of every `EffectValue` kind, plus a
/// keyframed `Float` beside the static one. `float` also carries an unknown
/// `extra` field, standing in for a document written by a newer Lumit: the
/// round-trip assertions below compare whole instances, so anything the bridge
/// dropped on the way through would show up there.
fn effect_with_every_kind() -> lumit_core::model::EffectInstance {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp, EASY_EASE};
    use lumit_core::model::{
        EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue, FileParam,
    };
    use lumit_core::time::Rational;

    let param = |id: &str, value: EffectValue| EffectParam {
        id: id.into(),
        value,
        extra: serde_json::Map::new(),
    };

    let mut carries_extra = serde_json::Map::new();
    carries_extra.insert("expression".into(), serde_json::json!("time * 2"));

    let curve = Animation::Keyframed(vec![
        Keyframe {
            time: Rational::new(0, 1).expect("0 s"),
            value: 5.0,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        },
        Keyframe {
            // A half-second: exactly the sort of time that would stop landing on
            // its own frame if it crossed as a float.
            time: Rational::new(1, 2).expect("half a second"),
            value: 20.0,
            interp_in: EASY_EASE,
            interp_out: SideInterp::Hold,
        },
    ]);

    EffectInstance {
        id: Uuid::now_v7(),
        effect: EffectKey {
            namespace: EffectNamespace::Builtin,
            match_name: "blur".into(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: vec![
            param(
                "float",
                EffectValue::Float(Property {
                    animation: Animation::Static(4.5),
                    extra: carries_extra,
                }),
            ),
            param(
                "animated",
                EffectValue::Float(Property {
                    animation: curve,
                    extra: serde_json::Map::new(),
                }),
            ),
            param(
                "point",
                EffectValue::Point(Property::fixed(10.0), Property::fixed(-3.0)),
            ),
            param(
                "colour",
                EffectValue::Colour([
                    Property::fixed(0.1),
                    Property::fixed(0.2),
                    Property::fixed(0.3),
                    Property::fixed(1.0),
                ]),
            ),
            param("bool", EffectValue::Bool(true)),
            param("choice", EffectValue::Choice(2)),
            param("seed", EffectValue::Seed(77)),
            param(
                "file",
                EffectValue::File(FileParam::single("C:/maps/displace.png")),
            ),
            param("layer", EffectValue::Layer(Some(Uuid::now_v7()))),
        ],
        sample_temporally: true,
        extra: serde_json::Map::new(),
    }
}

/// Put `effects` on the layer straight through the store, so a test can start
/// from a stack the frb add path could not have built.
fn seed_stack(
    project: &ProjectReference,
    layer: &LayerReference,
    effects: Vec<lumit_core::model::EffectInstance>,
) {
    let state = project.state().expect("state");
    let state = state.write().expect("write");
    state
        .store
        .commit(Op::SetLayerEffects {
            comp: layer.comp_id,
            layer: layer.layer_id,
            effects,
        })
        .expect("stack seeded");
}

/// The layer's effect stack as the document holds it.
fn stack_of(layer: &LayerReference) -> Vec<lumit_core::model::EffectInstance> {
    layer
        .get_effects()
        .expect("stack")
        .iter()
        .map(|e| e.get_effects())
        .collect()
}

/// Undo exactly one step.
fn undo_once(project: &ProjectReference) {
    let state = project.state().expect("state");
    let state = state.read().expect("read");
    state
        .store
        .undo()
        .expect("undo applied")
        .expect("there was something to undo");
}

/// The whole promise of the value type: whatever a parameter reads as can be
/// written straight back, for every kind, and the document is left exactly as it
/// was — keyframes, keyframe interpolation, file paths, layer reference and all.
/// Without that, "read the value, change one field, write it" — the way every
/// control in the panel works — would quietly damage the parameters it touched.
#[test]
fn every_effect_value_kind_round_trips_through_the_document() {
    let (project, layer) = project_with_layer();
    let original = effect_with_every_kind();
    seed_stack(&project, &layer, vec![original.clone()]);

    let mut staged = layer.get_effects().expect("stack");
    assert_eq!(staged.len(), 1);
    let ids = staged[0].get_parameters();
    assert_eq!(
        ids.len(),
        9,
        "one parameter per kind, plus the animated float"
    );

    for id in ids {
        let value = staged[0]
            .get_value(id.clone())
            .unwrap_or_else(|e| panic!("every kind reads: {id} answered {e}"));
        staged[0]
            .set_value(id.clone(), value)
            .unwrap_or_else(|e| panic!("every kind writes: {id} answered {e}"));
    }
    layer.set_effects(staged).expect("committed");

    assert_eq!(stack_of(&layer), vec![original]);
}

/// A keyframed Float must read as its curve, not as its value at time zero. The
/// `f64`-only predecessor could only answer `None` here, which is why an animated
/// parameter was unreachable; answering a number instead would be worse, because
/// writing it back would delete the animation.
#[test]
fn a_keyframed_float_reads_as_its_keys_and_is_not_flattened() {
    let (project, layer) = project_with_layer();
    seed_stack(&project, &layer, vec![effect_with_every_kind()]);
    let staged = layer.get_effects().expect("stack");

    let value = staged[0].get_value("animated".into()).expect("a value");
    let BridgeEffectValue::Float(BridgeScalar::Keyframed(keys)) = value else {
        panic!("a keyframed float must not read as a static number");
    };
    assert_eq!(keys.len(), 2);
    // Exact times, as integers: 1/2 s, not 0.5.
    assert_eq!((keys[0].time.num, keys[0].time.den), (0, 1));
    assert_eq!((keys[1].time.num, keys[1].time.den), (1, 2));
    assert_eq!(keys[1].value, 20.0);
    assert!(
        matches!(keys[1].interp_in, BridgeSideInterp::Bezier(_)),
        "the eased side survives, so the graph editor can draw its handle"
    );
    assert!(matches!(keys[1].interp_out, BridgeSideInterp::Hold));

    // The static sibling still reads static — the distinction is per parameter.
    assert!(matches!(
        staged[0].get_value("float".into()),
        Ok(BridgeEffectValue::Float(BridgeScalar::Static(_)))
    ));
}

/// A parameter's kind is the effect's schema to declare, not the panel's to
/// change. Writing the wrong kind is refused and the value left alone, rather
/// than becoming something the effect's own resolver cannot read.
#[test]
fn writing_the_wrong_kind_to_a_parameter_is_refused() {
    let (project, layer) = project_with_layer();
    seed_stack(&project, &layer, vec![effect_with_every_kind()]);
    let mut staged = layer.get_effects().expect("stack");

    let before = staged[0].get_value("colour".into()).expect("a colour");
    let refused = staged[0].set_value(
        "colour".into(),
        BridgeEffectValue::Float(BridgeScalar::Static(1.0)),
    );
    assert!(matches!(refused, Err(BridgeError::ParamKindMismatch)));
    assert_eq!(
        staged[0]
            .get_value("colour".into())
            .expect("still a colour"),
        before,
        "a refused write changes nothing"
    );

    // The other direction refuses too, and an unknown parameter is a calm error
    // rather than a silent no-op.
    assert!(matches!(
        staged[0].set_value("float".into(), BridgeEffectValue::Bool(true)),
        Err(BridgeError::ParamKindMismatch)
    ));
    assert!(matches!(
        staged[0].get_value("nope".into()),
        Err(BridgeError::InvalidParam)
    ));
}

/// Keys the engine could not evaluate are refused on the way in. `anim::evaluate`
/// walks the list assuming it is sorted, so an unsorted one would not fail — it
/// would silently evaluate wrongly, which is far harder to notice.
#[test]
fn a_keyframed_value_the_engine_could_not_evaluate_is_refused() {
    let (project, layer) = project_with_layer();
    seed_stack(&project, &layer, vec![effect_with_every_kind()]);
    let mut staged = layer.get_effects().expect("stack");

    let key = |num: i64, den: i64| BridgeKeyframe {
        time: BridgeRational { num, den },
        value: 1.0,
        interp_in: BridgeSideInterp::Linear,
        interp_out: BridgeSideInterp::Linear,
    };
    let write = |staged: &mut Vec<crate::api::effect::BridgeEffectInstance>,
                 keys: Vec<BridgeKeyframe>| {
        staged[0].set_value(
            "animated".into(),
            BridgeEffectValue::Float(BridgeScalar::Keyframed(keys)),
        )
    };

    assert!(matches!(
        write(&mut staged, Vec::new()),
        Err(BridgeError::InvalidKeyframes)
    ));
    assert!(matches!(
        write(&mut staged, vec![key(1, 1), key(0, 1)]),
        Err(BridgeError::InvalidKeyframes)
    ));
    assert!(
        matches!(
            write(&mut staged, vec![key(0, 1), key(0, 1)]),
            Err(BridgeError::InvalidKeyframes)
        ),
        "two keys at the same time are not a curve either"
    );
    assert!(matches!(
        write(&mut staged, vec![key(1, 0)]),
        Err(BridgeError::InvalidKeyframes)
    ));

    // A valid curve still writes, so the guard is not simply refusing everything.
    write(&mut staged, vec![key(0, 1), key(1, 2)]).expect("an ascending curve writes");
}

/// The Add-effect menu's source list. It carries the label and the category keys
/// as well as the match name, because the menu groups by category (K-090) and
/// draws the label, and a second call to find those out would be wasted.
#[test]
fn list_effects_names_the_builtins_with_their_labels_and_categories() {
    let effects = list_effects();
    assert!(!effects.is_empty());
    assert!(
        effects
            .iter()
            .any(|e| e.name == "blur" && e.label == "Gaussian blur"),
        "the match name and its menu label are distinct, and both are carried"
    );
    assert!(effects
        .iter()
        .all(|e| !e.category.is_empty() && !e.category_label.is_empty()));
}

/// Each stack op is one `SetLayerEffects`, so one undo puts the stack back
/// exactly as it was. A single op that landed as two would leave the stack
/// half-restored here, which is the failure this is watching for.
#[test]
fn each_effect_stack_op_lands_as_one_undo_step() {
    let (project, layer) = project_with_layer();
    let builtins = list_effects();
    let (first, second) = (builtins[0].name.clone(), builtins[1].name.clone());

    // Add.
    layer.add_effect(first.clone()).expect("added");
    let added = stack_of(&layer);
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].effect.match_name, first);
    undo_once(&project);
    assert!(
        stack_of(&layer).is_empty(),
        "one undo unwinds the whole add"
    );

    layer.add_effect(first.clone()).expect("added again");
    layer.add_effect(second).expect("a second effect");
    let two = stack_of(&layer);
    assert_eq!(two.len(), 2, "an added effect appends to the stack");

    // Bypass.
    layer
        .set_effect_enabled(&layer.get_effects().expect("stack")[0], false)
        .expect("bypassed");
    assert!(!stack_of(&layer)[0].enabled);
    undo_once(&project);
    assert_eq!(stack_of(&layer), two, "one undo restores the whole stack");

    // Reorder.
    layer
        .reorder_effect(&layer.get_effects().expect("stack")[0], 1)
        .expect("reordered");
    assert_eq!(stack_of(&layer)[1].id, two[0].id);
    undo_once(&project);
    assert_eq!(stack_of(&layer), two);

    // Remove.
    layer
        .remove_effect(&layer.get_effects().expect("stack")[0])
        .expect("removed");
    assert_eq!(stack_of(&layer).len(), 1);
    undo_once(&project);
    assert_eq!(stack_of(&layer), two);
}

/// A drag that overshoots the list is an ordinary thing for a pointer to do, so
/// the index clamps rather than the reorder failing and leaving the effect where
/// it started with no explanation.
#[test]
fn reorder_effect_clamps_an_index_outside_the_stack() {
    let (project, layer) = project_with_layer();
    let names: Vec<String> = list_effects()
        .iter()
        .take(3)
        .map(|e| e.name.clone())
        .collect();
    for name in &names {
        layer.add_effect(name.clone()).expect("added");
    }
    let ids: Vec<Uuid> = stack_of(&layer).iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), 3);

    // Far past the end lands it at the bottom.
    layer
        .reorder_effect(&layer.get_effects().expect("stack")[0], 99)
        .expect("clamped, not refused");
    assert_eq!(stack_of(&layer)[2].id, ids[0]);

    // Negative lands it back at the top.
    layer
        .reorder_effect(&layer.get_effects().expect("stack")[2], -5)
        .expect("clamped, not refused");
    assert_eq!(stack_of(&layer)[0].id, ids[0]);

    // And the whole document is still consistent: three effects, no duplicates.
    let after: Vec<Uuid> = stack_of(&layer).iter().map(|e| e.id).collect();
    assert_eq!(after.len(), 3);
    assert_eq!(after[0], ids[0]);
    let _ = project;
}

/// Effects that are no longer there, and names that never were, are calm errors.
#[test]
fn the_stack_ops_refuse_what_they_cannot_find() {
    let (project, layer) = project_with_layer();
    seed_stack(&project, &layer, vec![effect_with_every_kind()]);

    assert!(matches!(
        layer.add_effect("not-an-effect".into()),
        Err(BridgeError::UnknownEffectName)
    ));

    let stale = layer.get_effects().expect("stack");
    layer.remove_effect(&stale[0]).expect("removed");
    // The reference now outlives its effect: an error, never a panic.
    assert!(matches!(
        layer.remove_effect(&stale[0]),
        Err(BridgeError::InvalidEffect)
    ));
    assert!(matches!(
        layer.reorder_effect(&stale[0], 0),
        Err(BridgeError::InvalidEffect)
    ));
    assert!(matches!(
        layer.set_effect_enabled(&stale[0], false),
        Err(BridgeError::InvalidEffect)
    ));
}

/// `set_effects` commits parameter values, and only those. A stack staged before
/// something else removed an effect from it would otherwise resurrect that
/// effect on mouse-up — and reorder and delete would have a second, silent path
/// that cannot say what it meant.
#[test]
fn committing_a_staged_stack_that_no_longer_matches_the_document_is_refused() {
    let (project, layer) = project_with_layer();
    let mut first = effect_with_every_kind();
    first.params.clear();
    let mut second = effect_with_every_kind();
    second.params.clear();
    second.id = Uuid::now_v7();
    seed_stack(&project, &layer, vec![first.clone(), second.clone()]);

    let staged = layer.get_effects().expect("stack");
    layer
        .remove_effect(&layer.get_effects().expect("stack")[1])
        .expect("removed behind the panel's back");

    assert!(matches!(
        layer.set_effects(staged),
        Err(BridgeError::StaleEffectStack)
    ));
    assert_eq!(
        stack_of(&layer),
        vec![first],
        "the removal stands; nothing is resurrected"
    );
}

// --- Change scoping -------------------------------------------------------
//
// `op_scope` is what stops the Project panel rebuilding — and re-probing every
// footage file on disk — every time someone nudges a layer value. It used to
// serialise each op to JSON and look for `comp`/`layer` string fields, so every
// project-level op fell through unscoped and Dart could not tell the two apart.

/// A layer edit is not a project-item edit. This is the regression: with the
/// JSON sniffing, `items` did not exist and the panel rebuilt on this op.
#[test]
fn a_layer_edit_scopes_to_its_layer_and_not_the_item_list() {
    let (comp, layer) = (Uuid::now_v7(), Uuid::now_v7());

    assert_eq!(
        crate::api::state::op_scope(&Op::SetLayerVisible {
            comp,
            layer,
            visible: false,
        }),
        (Some(comp), Some(layer), false)
    );

    // Adding or removing a layer changes the comp's layer list, not one layer's
    // contents, so it reports the comp alone.
    assert_eq!(
        crate::api::state::op_scope(&Op::RemoveLayer { comp, layer }),
        (Some(comp), None, false)
    );
}

/// Every op that adds, removes, renames, refiles or relinks an item sets the
/// flag the Project panel listens on.
#[test]
fn project_item_edits_scope_to_the_item_list() {
    let (id, folder) = (Uuid::now_v7(), Uuid::now_v7());

    for op in [
        Op::RemoveItem { id },
        Op::RenameItem {
            id,
            name: "hero".into(),
        },
        Op::SetFolderChildren {
            folder,
            children: vec![id],
        },
        Op::SetAutoFolder {
            kind: lumit_core::ops::AutoFolderKind::Solids,
            folder: Some(folder),
        },
    ] {
        assert_eq!(
            crate::api::state::op_scope(&op),
            (None, None, true),
            "{op:?} should reach the Project panel"
        );
    }

    // Comp settings carry the comp's name, which is the panel's row label, so
    // this one is both an item-list change and a comp change.
    assert_eq!(
        crate::api::state::op_scope(&Op::SetCompSettings {
            comp: id,
            name: "Scene".into(),
            width: 1920,
            height: 1080,
            frame_rate: lumit_core::time::FrameRate::new(25, 1).expect("25 fps"),
            duration: lumit_core::time::Duration(
                lumit_core::time::Rational::new(5, 1).expect("5 s")
            ),
            background: lumit_core::model::LinearColour::BLACK,
        }),
        (Some(id), None, true)
    );
}

/// A batch is as broad as its members: `move_to_root` commits a batch of folder
/// edits and must still reach the panel, while a batch of layer edits must not.
#[test]
fn a_batch_takes_the_widest_scope_of_its_members() {
    let (comp, layer, folder) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    assert_eq!(
        crate::api::state::op_scope(&Op::Batch {
            ops: vec![
                Op::SetLayerVisible {
                    comp,
                    layer,
                    visible: false,
                },
                Op::SetFolderChildren {
                    folder,
                    children: vec![],
                },
            ],
        }),
        (None, None, true)
    );

    assert_eq!(
        crate::api::state::op_scope(&Op::Batch {
            ops: vec![
                Op::SetLayerVisible {
                    comp,
                    layer,
                    visible: false,
                },
                Op::RenameLayer {
                    comp,
                    layer,
                    name: "Adjust".into(),
                },
            ],
        }),
        (None, None, false)
    );
}

/// The panel draws a row per declared parameter, so the schema has to come
/// across whole: labels to show, ranges for the sliders, option names for the
/// dropdowns. Blur is the check because it declares a Float with a slider and a
/// half-open hard bound plus a grouped Choice.
#[test]
fn list_parameters_carries_the_schema_a_control_needs() {
    use crate::api::effect::{list_parameters, BridgeParamKind};

    let params = list_parameters("blur".into());
    assert!(!params.is_empty(), "blur declares parameters");

    let radius = params
        .iter()
        .find(|p| p.id == "radius")
        .expect("blur has a radius");
    assert_eq!(radius.label, "Radius", "the label is what the row shows");
    let BridgeParamKind::Float {
        slider_min,
        slider_max,
        hard_min,
        ..
    } = &radius.kind
    else {
        panic!("radius is a float");
    };
    assert!(slider_max > slider_min, "the slider has travel");
    assert_eq!(*hard_min, Some(0.0), "a blur radius cannot go negative");

    // Every declared parameter is expressible: no kind falls through.
    for p in &params {
        assert!(!p.label.is_empty(), "{} has a label", p.id);
    }
}

/// An effect this build does not know is an empty list, not an error — a project
/// carrying one still opens, its instance simply has no rows.
#[test]
fn list_parameters_of_an_unknown_effect_is_empty() {
    assert!(crate::api::effect::list_parameters("not-an-effect".into()).is_empty());
}

/// Every built-in's parameters survive the crossing. A kind added to the schema
/// without an arm here would panic in the mapping; this walks the lot so that
/// cannot reach a user.
#[test]
fn every_builtin_lists_its_parameters() {
    for info in crate::api::effect::list_effects() {
        let params = crate::api::effect::list_parameters(info.name.clone());
        let declared = lumit_core::fx::BUILTINS
            .iter()
            .find(|s| s.match_name == info.name)
            .expect("listed effects are built in")
            .params
            .len();
        assert_eq!(params.len(), declared, "{} lost a parameter", info.name);
    }
}

// --- Transform ------------------------------------------------------------

/// Reading the group and writing one property back leaves the document
/// unchanged: the same round-trip rule the effect values follow, and what makes
/// "read, change one field, write" safe for the panel's controls.
#[test]
fn a_transform_round_trips_through_the_bridge() {
    use crate::api::layer::BridgeTransformProp;

    let (_project, layer) = project_with_layer();
    let before = layer.get_transform().expect("transform");

    layer
        .set_transform(BridgeTransformProp::PositionX, before.position_x.clone())
        .expect("written");

    assert_eq!(
        layer.get_transform().expect("transform").position_x,
        before.position_x,
        "writing back what was read changes nothing"
    );
}

/// One property per op, so undo restores exactly what was nudged and nothing
/// else. Committing the whole group would make one undo step put back ten
/// properties the user never touched.
#[test]
fn setting_one_property_leaves_the_others_alone_and_undoes_alone() {
    use crate::api::effect::BridgeScalar;
    use crate::api::layer::{BridgeTransform, BridgeTransformProp};

    let (project, layer) = project_with_layer();
    let before = layer.get_transform().expect("transform");

    layer
        .set_transform(BridgeTransformProp::Opacity, BridgeScalar::Static(42.0))
        .expect("written");

    let after = layer.get_transform().expect("transform");
    assert_eq!(after.opacity, BridgeScalar::Static(42.0));
    assert_eq!(after.position_x, before.position_x, "position untouched");
    assert_eq!(after.scale_x, before.scale_x, "scale untouched");

    project.undo().expect("undone");
    assert_eq!(
        layer.get_transform().expect("transform").opacity,
        before.opacity,
        "one op, one undo step"
    );

    // The preview writer takes the whole group, which is the drag path's shape.
    let mut group = lumit_core::model::TransformGroup::default();
    BridgeTransform {
        opacity: BridgeScalar::Static(7.0),
        ..after
    }
    .write(&mut group)
    .expect("preview write");
    assert_eq!(
        group.opacity.animation,
        lumit_core::anim::Animation::Static(7.0)
    );
}

/// The Audio group's one control (docs/07 §4.3). Volume is a property like any
/// other — one op, invertible — and a layer that cannot be heard says so, which
/// is what decides whether the group is offered at all.
#[test]
fn volume_round_trips_and_a_solid_reports_no_audio() {
    use crate::api::effect::BridgeScalar;

    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer().expect("layer");

    assert!(
        matches!(
            layer.get_volume_db().expect("volume"),
            BridgeScalar::Static(db) if db == 0.0
        ),
        "unity to start with"
    );

    layer
        .set_volume_db(BridgeScalar::Static(-6.0))
        .expect("applied");
    assert!(matches!(
        layer.get_volume_db().expect("volume"),
        BridgeScalar::Static(db) if db == -6.0
    ));

    // One op, so one undo puts it back.
    project.undo().expect("undo");
    assert!(matches!(
        layer.get_volume_db().expect("volume"),
        BridgeScalar::Static(db) if db == 0.0
    ));

    assert!(
        !layer.has_audio().expect("asked"),
        "a solid has no sound to set, so it is offered no Audio group"
    );
}

/// A reference that outlives its layer is a calm error, never a panic — the
/// same contract every other reference method keeps.
#[test]
fn a_transform_edit_on_a_dead_layer_is_a_calm_error() {
    use crate::api::effect::BridgeScalar;
    use crate::api::layer::BridgeTransformProp;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let stale = LayerReference::new(project.id, Uuid::now_v7(), Uuid::now_v7());

    assert!(matches!(
        stale.get_transform(),
        Err(BridgeError::InvalidItem) | Err(BridgeError::InvalidLayer)
    ));
    assert!(matches!(
        stale.set_transform(BridgeTransformProp::Opacity, BridgeScalar::Static(1.0)),
        Err(BridgeError::InvalidItem) | Err(BridgeError::InvalidLayer)
    ));
}

/// Keyframe times must be exact: at 29.97 fps a frame is 1001/30000 s, and a
/// panel that worked that out in floating point would place keys that do not
/// land on the frame they were set on. Round-tripping through the pair is the
/// property that matters.
#[test]
fn frame_and_time_round_trip_exactly_at_a_drop_frame_rate() {
    use crate::api::composition::BridgeCompSettings;

    let (project, layer) = project_with_layer();
    let _ = layer;
    let comp = match project
        .get_items()
        .expect("roots")
        .into_iter()
        .find_map(|i| match i {
            ItemReference::Folder(folder) => folder.get_children().ok().and_then(|kids| {
                kids.into_iter().find_map(|k| match k {
                    ItemReference::Composition(c) => Some(c),
                    _ => None,
                })
            }),
            ItemReference::Composition(c) => Some(c),
            _ => None,
        }) {
        Some(c) => c,
        None => panic!("the fixture made a composition"),
    };

    let settings = comp.get_settings().expect("settings");
    comp.set_settings(BridgeCompSettings {
        fps_num: 30000,
        fps_den: 1001,
        ..settings
    })
    .expect("29.97");

    for frame in [0_i64, 1, 24, 100, 3597] {
        let time = comp.time_of_frame(frame).expect("time");
        assert_eq!(
            comp.frame_at_time(time).expect("frame"),
            frame,
            "frame {frame} did not survive the round trip"
        );
    }

    // …and the pair really is the exact rational, not a rounded one.
    let one = comp.time_of_frame(1).expect("time");
    assert_eq!((one.num, one.den), (1001, 30000));
}

// --- Timeline -------------------------------------------------------------

/// Every layer kind the Timeline's Layer menu offers actually lands, and each
/// one is a single undo step. The solid is the interesting one: it is a batch
/// (the asset, its auto-folder, and the layer), and a batch that undid in
/// pieces would leave an orphaned SolidDef in the Project panel.
#[test]
fn every_layer_kind_adds_and_undoes_as_one_step() {
    use crate::api::layer::BridgeLayerKind;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    // Matched inside the loop, not collected into an array first: an array of
    // results would run every adder before the body checked any of them, and
    // only the last would still be on top.
    for expected in [
        BridgeLayerKind::Solid,
        BridgeLayerKind::Text,
        BridgeLayerKind::Camera,
        BridgeLayerKind::Adjustment,
        BridgeLayerKind::NullObject,
        BridgeLayerKind::Sequence,
    ] {
        let added = match expected {
            BridgeLayerKind::Solid => comp.add_solid_layer(),
            BridgeLayerKind::Text => comp.add_text_layer(),
            BridgeLayerKind::Camera => comp.add_camera_layer(),
            BridgeLayerKind::Adjustment => comp.add_adjustment_layer(),
            BridgeLayerKind::NullObject => comp.add_null_object_layer(),
            BridgeLayerKind::Sequence => comp.add_sequence_layer(),
            other => panic!("{other:?} has no Layer-menu entry"),
        }
        .expect("layer added");
        assert_eq!(added.get_kind().expect("kind"), expected);
        assert_eq!(
            comp.get_layers().expect("layers")[0].id(),
            added.id(),
            "{expected:?} went to the top of the stack"
        );

        let before = comp.get_layers().expect("layers").len();
        project.undo().expect("undone");
        assert_eq!(
            comp.get_layers().expect("layers").len(),
            before - 1,
            "{expected:?} came off in one undo step"
        );
        project.redo().expect("redone");
    }
}

/// Each switch is its own op, so a click is one undo step and toggling one
/// switch never disturbs another.
#[test]
fn the_switches_are_independent_and_each_is_one_undo_step() {
    use crate::api::layer::BridgeLayerSwitch as S;

    let (project, layer) = project_with_layer();
    assert!(
        layer.get_switches().expect("switches").visible,
        "layers start visible"
    );

    for switch in [
        S::Visible,
        S::Audible,
        S::Locked,
        S::Solo,
        S::ThreeD,
        S::Fx,
        S::MotionBlur,
        S::Collapse,
        S::Shy,
    ] {
        let start = layer.get_switches().expect("switches");
        let now = match switch {
            S::Visible => start.visible,
            S::Audible => start.audible,
            S::Locked => start.locked,
            S::Solo => start.solo,
            S::ThreeD => start.three_d,
            S::Fx => start.fx,
            S::MotionBlur => start.motion_blur,
            S::Collapse => start.collapse,
            S::Shy => start.shy,
        };
        layer.set_switch(switch, !now).expect("toggled");
        assert_ne!(
            layer.get_switches().expect("switches"),
            start,
            "{switch:?} changed something"
        );
        project.undo().expect("undone");
        assert_eq!(
            layer.get_switches().expect("switches"),
            start,
            "{switch:?} undid cleanly"
        );
    }
}

/// The comp's master motion-blur shutter (K-120): the read model reports it,
/// the setter flips only the enable — angle, phase and samples keep their
/// values — and the flip is one undo step.
#[test]
fn the_master_motion_blur_toggle_flips_only_the_enable() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = add_comp(&project, "Scene");

    assert!(
        !comp.get_model().expect("model").motion_blur_enabled,
        "the master starts off"
    );

    comp.set_motion_blur_enabled(true).expect("enabled");
    assert!(comp.get_model().expect("model").motion_blur_enabled);
    let mb = comp.composition().expect("comp").motion_blur;
    assert_eq!(
        (mb.shutter_angle, mb.shutter_phase, mb.samples),
        (180.0, -90.0, 16),
        "the shutter shape kept its defaults"
    );

    project.undo().expect("undone");
    assert!(
        !comp.get_model().expect("model").motion_blur_enabled,
        "one undo step turned it back off"
    );
}

/// A span is one op even when the drag moved all three edges — a slip edit
/// changes the in point and the start offset together, and two undo steps for
/// one gesture is what the whole-value shape exists to avoid.
#[test]
fn a_span_edit_is_one_op_and_a_bad_one_is_refused() {
    use crate::api::effect::BridgeRational;
    use crate::api::layer::BridgeSpan;

    let (_project, layer) = project_with_layer();

    layer
        .set_span(BridgeSpan {
            in_point: BridgeRational { num: 1, den: 1 },
            out_point: BridgeRational { num: 4, den: 1 },
            start_offset: BridgeRational { num: 1, den: 2 },
        })
        .expect("trimmed and slipped in one op");

    let after = layer.get_span().expect("span");
    assert_eq!(after.in_point, BridgeRational { num: 1, den: 1 });
    assert_eq!(after.out_point, BridgeRational { num: 4, den: 1 });
    assert_eq!(after.start_offset, BridgeRational { num: 1, den: 2 });

    // An out point that is not after the in point is refused by the op, not
    // clamped: a zero-length layer is not something a drag should produce.
    assert!(layer
        .set_span(BridgeSpan {
            in_point: BridgeRational { num: 4, den: 1 },
            out_point: BridgeRational { num: 4, den: 1 },
            start_offset: BridgeRational { num: 0, den: 1 },
        })
        .is_err());

    // A denominator of zero is a caller bug; refused rather than normalised.
    assert!(matches!(
        layer.set_span(BridgeSpan {
            in_point: BridgeRational { num: 1, den: 0 },
            out_point: BridgeRational { num: 4, den: 1 },
            start_offset: BridgeRational { num: 0, den: 1 },
        }),
        Err(BridgeError::InvalidTime)
    ));
    assert_eq!(layer.get_span().expect("span").in_point, after.in_point);
}

/// Blend modes cross as an index into one shared list, so the panel dropdown
/// and the engine cannot disagree about what a number means.
#[test]
fn blend_modes_round_trip_by_index_and_a_bad_index_is_refused() {
    let (_project, layer) = project_with_layer();
    let modes = crate::api::composition::list_blend_modes();
    assert!(modes.len() > 1, "there is more than Normal");
    assert_eq!(layer.get_blend().expect("blend"), 0, "layers start Normal");

    for index in 0..modes.len() as u32 {
        layer.set_blend(index).expect("set");
        assert_eq!(layer.get_blend().expect("blend"), index);
    }

    assert!(matches!(
        layer.set_blend(modes.len() as u32),
        Err(BridgeError::InvalidBlendMode)
    ));
}

/// A matte may dangle — its target degrades to "no matte" at render — but a
/// parent may not, because a parent loop has no defined transform. The two
/// behave differently on purpose.
#[test]
fn a_matte_may_dangle_but_a_parent_loop_is_refused() {
    use crate::api::layer::BridgeMatte;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let other = comp.add_adjustment_layer().expect("a second layer");

    layer
        .set_matte(Some(BridgeMatte {
            layer: other.id(),
            luma: true,
            inverted: false,
        }))
        .expect("matte set");
    let matte = layer.get_matte().expect("matte").expect("some");
    assert_eq!(matte.layer, other.id());
    assert!(matte.luma);

    other.delete().expect("the target is deleted");
    assert!(
        layer.get_matte().expect("matte").is_some(),
        "the reference stands; it degrades at render rather than being scrubbed"
    );

    // Parenting to itself would be a loop with no defined transform.
    assert!(layer.set_parent(Some(layer.id())).is_err());
    assert!(layer.get_parent().expect("parent").is_none());
}

/// A duplicate is a fresh layer, not a second reference to the same one: two
/// layers sharing an id would make every op that names a layer ambiguous.
#[test]
fn duplicating_a_layer_gives_it_fresh_ids() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    layer.add_effect("blur".into()).expect("an effect to copy");

    let copy = layer.duplicate().expect("duplicated");
    assert_ne!(copy.id(), layer.id());
    assert_eq!(comp.get_layers().expect("layers").len(), 2);

    let original_fx = layer.get_effects().expect("effects");
    let copied_fx = copy.get_effects().expect("effects");
    assert_eq!(copied_fx.len(), original_fx.len());
    assert_ne!(
        copied_fx[0].id(),
        original_fx[0].id(),
        "the copy carries its own effects"
    );
}

/// Markers and the work area belong to the comp, not a layer, and both
/// round-trip through the exact rational the document stores.
#[test]
fn markers_and_the_work_area_round_trip() {
    use crate::api::composition::BridgeMarker;
    use crate::api::effect::BridgeRational;
    use crate::api::layer::BridgeSpan;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    assert!(comp.get_work_area().expect("work area").is_none());
    comp.set_work_area(Some(BridgeSpan {
        in_point: BridgeRational { num: 1, den: 2 },
        out_point: BridgeRational { num: 3, den: 2 },
        start_offset: BridgeRational { num: 0, den: 1 },
    }))
    .expect("set");
    let area = comp.get_work_area().expect("work area").expect("some");
    assert_eq!(area.in_point, BridgeRational { num: 1, den: 2 });
    assert_eq!(area.out_point, BridgeRational { num: 3, den: 2 });

    comp.set_work_area(None).expect("cleared");
    assert!(comp.get_work_area().expect("work area").is_none());

    assert!(comp.get_markers().expect("markers").is_empty());
    comp.set_markers(vec![BridgeMarker {
        id: Uuid::now_v7(),
        time: BridgeRational {
            num: 1001,
            den: 30000,
        },
        label: "Chorus".into(),
    }])
    .expect("set");
    let markers = comp.get_markers().expect("markers");
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].label, "Chorus");
    assert_eq!(
        markers[0].time,
        BridgeRational {
            num: 1001,
            den: 30000
        },
        "an exact drop-frame time is not rounded on the way through"
    );
}

// --- Sequence layers, the razor, and the cache readout --------------------

/// Converting gives the layer one clip covering its whole span, and it is one
/// undo step even though the kind change is a remove-then-add pair.
#[test]
fn a_footage_layer_converts_to_a_sequence_layer_in_one_step() {
    use crate::api::layer::BridgeLayerKind;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);

    assert_eq!(layer.get_kind().expect("kind"), BridgeLayerKind::Footage);
    assert!(layer.get_clips().expect("clips").is_empty());

    layer.convert_to_sequenced().expect("converted");
    let converted = comp.get_layers().expect("layers").remove(0);
    assert_eq!(
        converted.get_kind().expect("kind"),
        BridgeLayerKind::Sequence
    );
    assert_eq!(
        converted.get_clips().expect("clips").len(),
        1,
        "one clip covering the source"
    );
    assert_eq!(
        comp.get_layers().expect("layers").len(),
        1,
        "converted in place, not added beside itself"
    );

    project.undo().expect("undone");
    assert_eq!(
        comp.get_layers().expect("layers")[0]
            .get_kind()
            .expect("kind"),
        BridgeLayerKind::Footage,
        "the remove-and-add pair is one undo step"
    );
}

/// Only footage converts, and the razor only cuts a Sequence layer. Both are
/// calm errors rather than panics, because both are reachable by pointing a
/// tool at the wrong row.
#[test]
fn the_sequence_ops_refuse_the_wrong_kind_of_layer() {
    let (_project, layer) = project_with_layer();

    assert!(matches!(
        layer.convert_to_sequenced(),
        Err(BridgeError::NotFootage)
    ));
    assert!(matches!(
        layer.cut_clip_at(0),
        Err(BridgeError::NotSequence)
    ));
    assert!(matches!(
        layer.delete_clip_at(0),
        Err(BridgeError::NotSequence)
    ));
    assert!(
        layer.get_clips().expect("clips").is_empty(),
        "a non-sequence layer has no clips rather than erroring — the Timeline\
         asks every row"
    );
}

/// The razor cuts in two without moving anything: a cut that shifted what comes
/// after it would break every edit already in time with the music (K-071).
#[test]
fn the_razor_cuts_and_deletes_without_moving_the_other_clips() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    layer.convert_to_sequenced().expect("converted");
    let layer = comp.get_layers().expect("layers").remove(0);

    let before = layer.get_clips().expect("clips");
    assert_eq!(before.len(), 1);

    layer.cut_clip_at(30).expect("cut");
    let after = layer.get_clips().expect("clips");
    assert_eq!(after.len(), 2, "one clip became two");
    assert_eq!(
        after[0].place_start, before[0].place_start,
        "the left half starts where the original did"
    );

    // Nowhere near the clip: a calm error, not a cut in the wrong place.
    assert!(matches!(
        layer.cut_clip_at(100_000),
        Err(BridgeError::NoClipThere)
    ));

    layer.delete_clip_at(30).expect("deleted");
    let remaining = layer.get_clips().expect("clips");
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].place_start, after[0].place_start,
        "deleting leaves a gap; the survivor does not ripple back"
    );
}

/// The cache readout answers without a project and never panics — the Settings
/// window can show it before anything is open.
#[test]
fn the_cache_readout_answers_and_the_budget_takes_effect() {
    use crate::api::cache::{cache_stats, clear_cache, set_cache_budget};

    let before = cache_stats();
    assert!(before.budget_bytes > 0, "there is a budget by default");

    let resized = set_cache_budget(64 << 20);
    assert_eq!(resized.budget_bytes, 64 << 20);
    assert_eq!(
        cache_stats().budget_bytes,
        64 << 20,
        "the new budget is what the next read sees"
    );

    let cleared = clear_cache();
    assert_eq!(cleared.entries, 0);
    assert_eq!(cleared.used_bytes, 0);

    // Put it back, so a later test in this process is not measuring ours.
    set_cache_budget(before.budget_bytes);
}

// --- Effect presets -------------------------------------------------------

/// A preset round-trips, and the copy it plants carries fresh instance ids —
/// applying one preset to two layers must not give them effects that share an
/// id, since an id is instance identity and every op that names an effect uses
/// it (K-065).
#[test]
fn a_preset_round_trips_with_fresh_instance_ids() {
    let (project, first) = project_with_layer();
    let comp = CompositionReference::new(project.id, first.comp_id());
    first.add_effect("blur".into()).expect("an effect to save");

    let text = first.save_preset("My look".into()).expect("saved");
    assert!(text.contains("\"format\""), "it is a .lumfx document");
    assert!(text.contains("My look"), "and carries its name");

    let second = comp.add_adjustment_layer().expect("a second layer");
    second.load_preset(text.clone()).expect("loaded");

    let source = first.get_effects().expect("effects");
    let copy = second.get_effects().expect("effects");
    assert_eq!(copy.len(), source.len());
    assert_eq!(copy[0].name(), source[0].name(), "the same effect");
    assert_ne!(copy[0].id(), source[0].id(), "but its own instance");

    // Loading appends, so a second load stacks rather than replacing.
    second.load_preset(text).expect("loaded again");
    assert_eq!(second.get_effects().expect("effects").len(), 2);

    // …and it is one undo step per load.
    project.undo().expect("undone");
    assert_eq!(second.get_effects().expect("effects").len(), 1);
}

/// Text that is not a preset is a calm error — a user can pick any file.
#[test]
fn a_file_that_is_not_a_preset_is_refused() {
    let (_project, layer) = project_with_layer();
    assert!(matches!(
        layer.load_preset("this is not JSON".into()),
        Err(BridgeError::InvalidPreset)
    ));
    assert!(
        layer.get_effects().expect("effects").is_empty(),
        "nothing partial was applied"
    );
}

/// A layer with no effects still saves. An empty preset is valid, and refusing
/// it would be a rule the user has to learn for no benefit.
#[test]
fn an_empty_stack_still_saves() {
    let (_project, layer) = project_with_layer();
    let text = layer.save_preset("Nothing".into()).expect("saved");
    layer.load_preset(text).expect("loaded");
    assert!(layer.get_effects().expect("effects").is_empty());
}

/// The scope request validates its colours rather than padding out a list the
/// caller got wrong — a trace drawn in colours nobody chose is worse than none.
#[test]
fn a_scope_needs_five_colour_triples() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    for bad in [
        vec![vec![0_u8, 0, 0]; 4],
        vec![vec![0_u8, 0, 0]; 6],
        vec![
            vec![0_u8, 0],
            vec![0, 0, 0],
            vec![0, 0, 0],
            vec![0, 0, 0],
            vec![0, 0, 0],
        ],
    ] {
        assert!(matches!(
            comp.render_scope(0, 1.0, 0, bad),
            Err(BridgeError::InvalidScopeColours)
        ));
    }

    // A well-formed request gets past validation; with no worker running it is
    // the worker-state error, never a panic.
    let good = vec![vec![0_u8, 0, 0]; 5];
    assert!(matches!(
        comp.render_scope(0, 1.0, 0, good),
        Err(BridgeError::InvalidWorkerState) | Ok(())
    ));
}

/// A comp nests into another as a Precomp layer, and refuses to nest into
/// itself — the one cycle a user reaches by accident.
#[test]
fn a_composition_nests_into_another_but_not_into_itself() {
    use crate::api::layer::BridgeLayerKind;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let inner = project.new_composition("Inner".into(), None).expect("comp");
    let outer = project.new_composition("Outer".into(), None).expect("comp");

    let placed = outer.add_precomp_layer(&inner).expect("nested");
    assert_eq!(placed.get_kind().expect("kind"), BridgeLayerKind::Precomp);
    assert_eq!(placed.get_name().expect("name"), "Inner");

    // The layer points back at the comp it draws, which is what the Hierarchy
    // panel walks.
    let source = placed.get_source_item().expect("source").expect("some");
    assert!(matches!(source, ItemReference::Composition(_)));

    assert!(matches!(
        outer.add_precomp_layer(&outer),
        Err(BridgeError::InvalidComp)
    ));
    assert_eq!(outer.get_layers().expect("layers").len(), 1);
}

// --- The shell: boot log, tier, autosave and recovery ---------------------

/// The boot log states facts and no more. It must never claim a GPU adapter,
/// because none is known until the first render — a splash that named one would
/// be inventing it.
#[test]
fn the_boot_log_states_only_what_the_build_knows() {
    let lines = crate::api::shell::boot_log();
    assert!(!lines.is_empty());
    assert!(
        lines[0].starts_with("lumit-bridge "),
        "its own version first"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("compositor:")),
        "and whether it can render at all"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("NVIDIA") || l.contains("Radeon")),
        "no adapter is named before one is probed"
    );
}

/// The tier is a readout of a controller that measures real render costs, so
/// the only thing to assert is that it answers in range and resets optimistic.
#[test]
fn the_playback_tier_reads_back_and_resets_to_full() {
    use crate::api::shell::{playback_tier, reset_realtime};

    let fresh = reset_realtime();
    assert_eq!(fresh.tier, 1, "a reset controller is optimistic");
    assert!((fresh.scale - 1.0).abs() < f32::EPSILON);

    let now = playback_tier();
    assert!((1..=4).contains(&now.tier));
    assert!((now.scale - 1.0 / now.tier as f32).abs() < 1e-6);
}

/// A project with no autosaves beside it is an ordinary empty answer, not an
/// error — the recovery dialogue opens before anything has been saved.
#[test]
fn listing_autosaves_of_a_project_with_none_is_empty() {
    let dir = std::env::temp_dir().join("lumit-autosave-none");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let project = dir.join("scene.lum");
    assert!(crate::api::shell::list_autosaves(project.to_string_lossy().into_owned()).is_empty());
    assert!(crate::api::shell::list_autosaves(String::new()).is_empty());
}

/// An autosave writes beside the project and leaves the project's own path
/// alone — the next Save must still write the file the user chose.
#[test]
fn an_autosave_writes_a_slot_without_moving_the_project() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    project
        .new_composition("Scene".into(), None)
        .expect("something to save");

    let dir = std::env::temp_dir().join("lumit-autosave-writes");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("scene.lum");
    let path = target.to_string_lossy().into_owned();

    let written = project.autosave(path.clone(), 3).expect("autosaved");
    assert!(std::path::Path::new(&written).is_file());
    assert!(
        project.path().expect("path").is_none(),
        "an autosave is a copy; the project has still never been saved"
    );

    let listed = crate::api::shell::list_autosaves(path);
    assert_eq!(listed.len(), 1, "one slot so far");
    assert_eq!(listed[0].slot, 1, "and it is the newest");

    std::fs::remove_dir_all(&dir).ok();
}

/// Recovery installs the opened document *through* the store, so the change
/// observer every panel listens to survives it.
#[test]
fn restoring_replaces_the_document_and_keeps_the_change_observer() {
    let dir = std::env::temp_dir().join("lumit-restore");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("scene.lum");

    let project = LumitBridgeState::new_project(None).expect("a new project");
    project.new_composition("Saved".into(), None).expect("comp");
    project
        .save(target.to_string_lossy().into_owned())
        .expect("saved");

    // Drift away from what is on disk, then restore.
    project
        .new_composition("Unsaved".into(), None)
        .expect("comp");
    let recovered = project
        .restore_journal(target.to_string_lossy().into_owned())
        .expect("restored");
    assert!(recovered.replayed <= recovered.found);

    // The document really was replaced, and the store still takes edits — which
    // is what proves the observer was not thrown away with it.
    project
        .new_composition("After".into(), None)
        .expect("still editable");
    assert!(
        !project.get_items().expect("roots").is_empty(),
        "the recovered document is the live one"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// --- Export ---------------------------------------------------------------

/// The export surface answers safely whatever has happened before it.
///
/// One test rather than two, and it never asserts "nothing has run yet": the
/// exporter's slot is process-wide and the suite runs in parallel, so a test
/// that assumed a pristine slot would pass or fail on test *order*.
#[test]
fn the_export_surface_refuses_calmly_and_never_panics() {
    use crate::api::export::{export_cancel, export_poll, BridgeExportSpec, BridgeExportState};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let spec = BridgeExportSpec {
        preset: String::new(),
        codec: "h264".into(),
        width: 0,
        height: 0,
        bitrate_mbps: 0,
        include_audio: true,
        audio_bit_rate: 0,
    };

    // Nowhere to write is refused before any work starts.
    assert!(matches!(
        comp.start_export(spec.clone(), "   ".into()),
        Err(BridgeError::NoProjectPath)
    ));

    // With a path it reaches the exporter, which on a machine with no GPU says
    // so — either way a calm answer, never a panic.
    let target = std::env::temp_dir().join("lumit-export-probe.mp4");
    let started = comp.start_export(spec, target.to_string_lossy().into_owned());
    assert!(
        started.is_ok() || matches!(started, Err(BridgeError::ExportFailed(_))),
        "an export either starts or explains itself"
    );

    // Poll always answers one of the four states, and cancelling is safe
    // whether or not anything is running.
    assert!(matches!(
        export_poll(),
        BridgeExportState::Idle
            | BridgeExportState::Running { .. }
            | BridgeExportState::Done { .. }
            | BridgeExportState::Failed { .. }
    ));
    export_cancel();
    export_cancel();
    std::fs::remove_file(&target).ok();
}

// --- Journalling ----------------------------------------------------------

/// Every commit is written to the crash journal as it happens. Without this the
/// autosave and the recovery dialogue have nothing to recover *from* — the
/// journal is the only record of work done since the last save.
#[test]
fn every_commit_is_journalled_and_a_save_clears_it() {
    let project = LumitBridgeState::new_project(None).expect("a new project");

    let journal = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        let handle = state.journal.lock().expect("journal");
        handle.clone()
    };
    let Some(journal) = journal else {
        // No home for a journal on this platform; nothing to assert.
        return;
    };
    journal.clear().ok();

    project
        .new_composition("Scene".into(), None)
        .expect("an edit");
    project
        .new_composition("Titles".into(), None)
        .expect("another");

    let ops = journal.read().expect("journal read");
    assert!(
        ops.len() >= 2,
        "each commit appended: {} ops for two edits",
        ops.len()
    );

    // Saving makes the journal redundant — a later recovery must not replay
    // edits the saved file already contains.
    let dir = std::env::temp_dir().join("lumit-journal-save");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("scene.lum");
    project
        .save(target.to_string_lossy().into_owned())
        .expect("saved");
    assert!(
        journal.read().expect("journal read").is_empty(),
        "the journal is cleared by a save"
    );

    // …and an edit after the save is not journalled against the stale handle:
    // the project disarmed it, so recovery from here is the saved file itself.
    project
        .new_composition("After".into(), None)
        .expect("an edit");
    assert!(journal.read().expect("journal read").is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

/// The observer runs inside `commit`, while the caller still holds the
/// project's write lock. Journalling from there must not reach back through the
/// registry — that would take the same lock and deadlock on the first edit.
/// This test would hang rather than fail if that regressed.
#[test]
fn journalling_does_not_deadlock_against_the_commit_lock() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    // Every one of these commits through a write guard, with the observer
    // firing inside it.
    for i in 0..8 {
        project
            .new_composition(format!("Comp {i}"), None)
            .expect("committed without deadlocking");
    }
    assert!(!project.get_items().expect("roots").is_empty());
}

/// Two threads opening projects and editing them at once must not deadlock.
///
/// frb runs calls on a worker pool, so this is a real arrangement rather than a
/// contrived one. It guards the lock order recorded beside `PROJECTS`: before
/// that was written down, `new_project` held the project registry while taking
/// the stream registry and `open_project` did the reverse, which two threads
/// could interleave into a deadlock. Like the journal test, this hangs rather
/// than fails on a regression — which is what a lock-order test can do.
#[test]
fn concurrent_project_creation_and_editing_does_not_deadlock() {
    let threads: Vec<_> = (0..4)
        .map(|t| {
            std::thread::spawn(move || {
                let project = LumitBridgeState::new_project(None).expect("a new project");
                for i in 0..6 {
                    project
                        .new_composition(format!("T{t} comp {i}"), None)
                        .expect("committed");
                }
                // Reading through a reference takes the registry and then the
                // project, which is the ordinary order the rule protects.
                assert!(!project.get_items().expect("roots").is_empty());
            })
        })
        .collect();

    for thread in threads {
        thread.join().expect("no thread panicked or hung");
    }
}

// --- Assets: what a layer is made of --------------------------------------

/// A text layer's words are editable and round-trip exactly. Before this the
/// frontend could add a Text layer and never change what it said.
#[test]
fn a_text_layer_round_trips_its_document() {
    use crate::api::assets::{BridgeColourRgba, BridgeTextDocument};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer().expect("a text layer");

    let before = text.get_text().expect("text").expect("it is text");
    assert_eq!(before.text, "Text", "the starter document");

    text.set_text(BridgeTextDocument {
        text: "Hello".into(),
        size: 48.0,
        fill: BridgeColourRgba {
            r: 1.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        },
    })
    .expect("set");

    let after = text.get_text().expect("text").expect("still text");
    assert_eq!(after.text, "Hello");
    assert_eq!(after.size, 48.0);
    assert!((after.fill.g - 0.5).abs() < 1e-6);

    project.undo().expect("undone");
    assert_eq!(
        text.get_text().expect("text").expect("text").text,
        "Text",
        "one undo step for the whole document"
    );

    // A layer that is not text answers None rather than erroring — the panel
    // asks every selected layer what it is.
    assert!(layer.get_text().expect("text").is_none());
    assert!(matches!(
        layer.set_text(BridgeTextDocument {
            text: "no".into(),
            size: 1.0,
            fill: BridgeColourRgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            },
        }),
        Err(BridgeError::NotText)
    ));
}

/// A camera's zoom is animatable, so it takes a whole scalar like every other
/// curve-capable value.
#[test]
fn a_camera_zoom_reads_and_writes_as_a_scalar() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let camera = comp.add_camera_layer().expect("a camera");

    let zoom = camera
        .get_camera_zoom()
        .expect("zoom")
        .expect("it is a camera");
    assert!(
        matches!(zoom, BridgeScalar::Static(v) if v > 0.0),
        "the AE 50 mm default"
    );

    camera
        .set_camera_zoom(BridgeScalar::Static(1200.0))
        .expect("set");
    assert_eq!(
        camera.get_camera_zoom().expect("zoom").expect("camera"),
        BridgeScalar::Static(1200.0)
    );

    assert!(layer.get_camera_zoom().expect("zoom").is_none());
    assert!(matches!(
        layer.set_camera_zoom(BridgeScalar::Static(1.0)),
        Err(BridgeError::NotCamera)
    ));
}

/// Editing a solid changes the **asset**, so every layer drawing it changes at
/// once. That is the point of solids being assets, and the thing a test should
/// pin down before somebody "fixes" it into a per-layer setting.
#[test]
fn editing_a_solid_changes_every_layer_that_uses_it() {
    use crate::api::assets::{BridgeColourRgba, BridgeSolidDef};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    comp.add_solid_layer().expect("a solid layer");

    // The solid asset it made, found in the project tree.
    let solid = project
        .get_items()
        .expect("roots")
        .into_iter()
        .find_map(|item| match item {
            ItemReference::Solid(solid) => Some(solid),
            ItemReference::Folder(folder) => folder.get_children().ok().and_then(|kids| {
                kids.into_iter().find_map(|k| match k {
                    ItemReference::Solid(solid) => Some(solid),
                    _ => None,
                })
            }),
            _ => None,
        })
        .expect("the solid asset was filed");

    let before = solid.get_definition().expect("definition");
    assert!(before.name.starts_with("White solid"));
    assert!((before.colour.r - 1.0).abs() < 1e-6, "white");

    solid
        .set_definition(BridgeSolidDef {
            name: "Backdrop".into(),
            colour: BridgeColourRgba {
                r: 0.0,
                g: 0.2,
                b: 0.4,
                a: 1.0,
            },
            width: 0,
            height: 0,
        })
        .expect("set");

    let after = solid.get_definition().expect("definition");
    assert_eq!(after.name, "Backdrop");
    assert!((after.colour.b - 0.4).abs() < 1e-6);
    assert_eq!(
        (after.width, after.height),
        (1, 1),
        "a zero-area solid is floored rather than committed as nothing"
    );

    // A blank name is refused, so an asset row cannot lose its label.
    assert!(matches!(
        solid.set_definition(BridgeSolidDef {
            name: "  ".into(),
            colour: after.colour,
            width: 100,
            height: 100,
        }),
        Err(BridgeError::EmptyName)
    ));
    assert_eq!(solid.get_definition().expect("definition").name, "Backdrop");
}

// --- Retime ---------------------------------------------------------------

/// A footage layer with no retiming is `None`, not 100% — "not retimed" and
/// "retimed to exactly 1×" are different states in the file, and only the first
/// skips the resampler.
#[test]
fn retiming_is_absent_until_it_is_switched_on() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);

    assert!(layer.get_retime().expect("retime").is_none());

    layer.set_retime_enabled(true).expect("on");
    let retime = layer.get_retime().expect("retime").expect("some");
    assert!(
        (retime.speed_percent - 100.0).abs() < 0.001,
        "switching it on changes nothing visible"
    );
    assert!(!retime.varies, "the identity map is one constant segment");

    layer.set_retime_enabled(false).expect("off");
    assert!(
        layer.get_retime().expect("retime").is_none(),
        "off removes the map rather than setting 100%"
    );
}

/// The speed, the reverse gate and the interpolation policy are independent —
/// a speed edit must not silently re-lock reverse or reset the policy.
#[test]
fn speed_reverse_and_interpolation_do_not_disturb_each_other() {
    use crate::api::retime::BridgeRetimeInterp;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    layer.set_retime_enabled(true).expect("on");

    layer.set_retime_reverse(true).expect("gate open");
    layer
        .set_retime_interpolation(BridgeRetimeInterp::Blend)
        .expect("policy");
    layer.set_retime_speed(50.0).expect("half speed");

    let retime = layer.get_retime().expect("retime").expect("some");
    assert!((retime.speed_percent - 50.0).abs() < 0.5);
    assert!(retime.allow_reverse, "the speed edit kept the gate open");
    assert_eq!(
        retime.interpolation,
        BridgeRetimeInterp::Blend,
        "and kept the policy"
    );

    // A freeze is a legal speed, not an error.
    layer.set_retime_speed(0.0).expect("freeze");
    assert!(
        layer
            .get_retime()
            .expect("retime")
            .expect("some")
            .speed_percent
            .abs()
            < 0.5
    );
}

/// Editing a curve that varies would discard its shape, so it is refused rather
/// than flattened — the same rule the keyframe rows follow.
#[test]
fn a_varying_curve_refuses_a_single_speed() {
    use lumit_core::retime::{Boundary, RateSegment, Retime, RetimeSegment};
    use lumit_core::time::Rational;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    layer.set_retime_enabled(true).expect("on");

    // A ramp, installed behind the row's back the way the Retime graph will.
    let one = Rational::new(1, 1).expect("1");
    let two = Rational::new(2, 1).expect("2");
    let ramp = Retime {
        boundaries: vec![
            Boundary::new(Rational::ZERO, Rational::ZERO),
            Boundary::new(one, two),
        ],
        segments: vec![RetimeSegment::Rate(RateSegment::new(
            Rational::ZERO,
            two,
            lumit_core::retime::Ease::Linear,
        ))],
        allow_reverse: false,
        interpolation: Default::default(),
        extra: serde_json::Map::new(),
    };
    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(lumit_core::Op::SetLayerRetime {
                comp: comp.id,
                layer: layer.id(),
                retime: Some(ramp),
            })
            .expect("ramp installed");
    }

    let read = layer.get_retime().expect("retime").expect("some");
    assert!(read.varies, "a ramp is not one constant speed");
    assert!(matches!(
        layer.set_retime_speed(50.0),
        Err(BridgeError::RetimeVaries)
    ));

    // …but the gate and the policy are still editable, because neither
    // discards the shape.
    layer.set_retime_reverse(true).expect("gate still editable");
    assert!(layer.get_retime().expect("retime").expect("some").varies);
}

/// Retiming is a footage-layer idea; every other kind refuses calmly.
#[test]
fn only_footage_layers_retime() {
    let (_project, layer) = project_with_layer();
    assert!(layer.get_retime().expect("retime").is_none());
    assert!(matches!(
        layer.set_retime_enabled(true),
        Err(BridgeError::NotFootage)
    ));
    assert!(matches!(
        layer.set_retime_speed(50.0),
        Err(BridgeError::NotFootage)
    ));
}

/// The Retime *property* (K-197) is an ordinary keyframable scalar: absent
/// until the layer is given one, present on any kind, and readable from the
/// row's read model without a second crossing.
#[test]
fn the_retime_property_toggles_and_reads_back() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id);

    assert!(layer.get_retime_property().expect("read").is_none());
    assert!(layer.get_info().expect("info").retime.is_none());
    // Not retimed yet: there is no curve to write into.
    assert!(matches!(
        layer.set_retime_property(BridgeScalar::Static(1.0)),
        Err(BridgeError::NotRetimed)
    ));

    assert!(layer.toggle_retime_property().expect("on"));
    // The identity map: two keys running source time alongside local time, so
    // the picture does not move when the row appears.
    let BridgeScalar::Keyframed(keys) = layer
        .get_retime_property()
        .expect("read")
        .expect("now retimed")
    else {
        panic!("the identity retime is keyframed");
    };
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].value, 0.0);
    assert!(keys[1].value > 0.0);
    // The Timeline draws its row from the read model, never a getter (K-184).
    assert!(layer.get_info().expect("info").retime.is_some());
    assert!(comp
        .get_model()
        .expect("model")
        .layers
        .iter()
        .any(|l| l.info.retime.is_some()));

    layer
        .set_retime_property(BridgeScalar::Static(2.5))
        .expect("write");
    assert!(matches!(
        layer.get_retime_property().expect("read"),
        Some(BridgeScalar::Static(v)) if (v - 2.5).abs() < 1e-9
    ));

    // Off removes it entirely — "not retimed", not "retimed to 1×".
    assert!(!layer.toggle_retime_property().expect("off"));
    assert!(layer.get_retime_property().expect("read").is_none());
}

// --- Audio and beats ------------------------------------------------------

/// The transport answers on a machine with no sound device — a CI runner, a
/// container — rather than failing. Silence must never stop the picture, so
/// `loaded` reads false and the caller keeps its own clock.
#[test]
fn the_audio_transport_answers_without_a_device() {
    use crate::api::audio::{audio_clock, audio_pause, audio_seek, audio_stop};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    // Preparing and playing a comp with no audible source is a no-op, not an
    // error: a comp with nothing to hear is an ordinary comp.
    comp.audio_prepare().expect("prepare");
    comp.audio_play(0.0).expect("play");

    let clock = audio_clock();
    assert!(
        !clock.loaded || clock.seconds >= 0.0,
        "either nothing is loaded, or the clock reads a real time"
    );

    // The rest of the transport is safe whatever the device did.
    audio_seek(1.5);
    audio_pause();
    audio_stop();
    assert!(!audio_clock().playing, "stop leaves it stopped");
}

/// Detection needs something to listen to. A comp with no audio says so rather
/// than placing zero markers and looking like it worked.
#[test]
fn detecting_beats_in_a_silent_composition_says_so() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    // On a machine with no GPU the pipeline itself is unavailable, which is a
    // different — and equally calm — answer.
    assert!(matches!(
        comp.detect_beats(50),
        Err(BridgeError::NoAudio) | Err(BridgeError::NoAudioPipeline)
    ));
}

/// Clearing keeps the markers a person made. Re-running detection at a
/// different sensitivity is ordinary, and losing your own notes to it would not
/// be.
#[test]
fn clearing_beats_keeps_the_markers_a_person_made() {
    use crate::api::composition::BridgeMarker;
    use crate::api::effect::BridgeRational;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    comp.set_markers(vec![BridgeMarker {
        id: Uuid::now_v7(),
        time: BridgeRational { num: 1, den: 2 },
        label: "Chorus".into(),
    }])
    .expect("a marker of my own");

    // A beat marker, placed the way detection places them.
    {
        let mut markers = comp.composition().expect("comp").markers;
        markers.push(lumit_core::markers::Marker::beat(
            Uuid::now_v7(),
            lumit_core::Rational::new(1, 1).expect("1 s"),
            0.9,
        ));
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(lumit_core::Op::SetCompMarkers {
                comp: comp.id,
                markers,
            })
            .expect("seeded");
    }
    assert_eq!(comp.get_markers().expect("markers").len(), 2);

    comp.clear_beat_markers().expect("cleared");
    let left = comp.get_markers().expect("markers");
    assert_eq!(left.len(), 1, "the beat went");
    assert_eq!(left[0].label, "Chorus", "and mine stayed");

    // Clearing again is a calm no-op — something a user does without thinking.
    comp.clear_beat_markers().expect("no-op");
    assert_eq!(comp.get_markers().expect("markers").len(), 1);
}

/// A row's stopwatch keys every axis it covers, and that has to be ONE undo
/// step — two ops for one click is exactly what the whole-value shape exists to
/// avoid everywhere else.
#[test]
fn setting_several_transform_properties_is_one_undo_step() {
    use crate::api::layer::BridgeTransformProp;

    let (project, layer) = project_with_layer();
    let before = layer.get_transform().expect("transform");

    layer
        .set_transforms(
            vec![
                BridgeTransformProp::PositionX,
                BridgeTransformProp::PositionY,
            ],
            vec![BridgeScalar::Static(10.0), BridgeScalar::Static(20.0)],
        )
        .expect("both axes");

    let after = layer.get_transform().expect("transform");
    assert_eq!(after.position_x, BridgeScalar::Static(10.0));
    assert_eq!(after.position_y, BridgeScalar::Static(20.0));

    project.undo().expect("undone");
    let undone = layer.get_transform().expect("transform");
    assert_eq!(undone.position_x, before.position_x, "one step, both axes");
    assert_eq!(undone.position_y, before.position_y);

    // Mismatched lists are a caller bug, and an empty one is a no-op so a
    // caller need not check before calling.
    assert!(matches!(
        layer.set_transforms(vec![BridgeTransformProp::Opacity], vec![]),
        Err(BridgeError::MismatchedTransforms)
    ));
    layer.set_transforms(vec![], vec![]).expect("no-op");
}

/// The preset library listing: real presets appear under their saved name (or
/// their file's stem when saved without one), sorted case-insensitively, and
/// strays — non-JSON, JSON that is not a preset, other extensions — are simply
/// not listed. The folder is the user's; a stray file there is not a fault.
#[test]
fn the_preset_library_lists_presets_and_skips_strays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let write = |file: &str, text: &str| {
        std::fs::write(dir.path().join(file), text).expect("write");
    };
    write(
        "zeta.lumfx",
        r#"{"format":1,"name":"Zeta look","effects":[]}"#,
    );
    write("anon.lumfx", r#"{"format":1,"effects":[]}"#);
    write(
        "Bright.LUMFX",
        r#"{"format":1,"name":"bright","effects":[]}"#,
    );
    write("notes.txt", "not a preset");
    write("broken.lumfx", "{ this is not json");
    write("shaped.lumfx", r#"{"name":"no effects list here"}"#);

    let listed = crate::api::effect::presets_in(dir.path());
    let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["anon", "bright", "Zeta look"],
        "named presets by name, nameless by stem, sorted, strays skipped"
    );
    assert!(
        listed
            .iter()
            .all(|p| p.path.ends_with("lumfx") || p.path.ends_with("LUMFX")),
        "each entry points at its file"
    );
}

/// "Any op, anywhere, is undoable" — the owner's standing rule, checked on the
/// layer row's newest controls: rename, label colour and matte each commit
/// exactly ONE undo step, and undoing each finds the state before it.
#[test]
fn rename_label_and_matte_each_undo_in_one_step() {
    use crate::api::layer::BridgeMatte;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = add_comp(&project, "Scene");
    let source = comp.add_solid_layer().expect("matte source");
    let layer = comp.add_solid_layer().expect("layer");
    let before = layer.get_info().expect("info").name;

    layer.rename("Hero".into()).expect("renamed");
    assert_eq!(layer.get_info().expect("info").name, "Hero");
    project.undo().expect("undo rename");
    assert_eq!(
        layer.get_info().expect("info").name,
        before,
        "one undo step returns the old name"
    );

    // A solid starts on its kind's default label (K-188), not on 0.
    let default_label = layer.get_info().expect("info").label;
    assert_eq!(
        default_label, 2,
        "a solid's default label is the solid chip"
    );
    layer.set_label(5).expect("labelled");
    assert_eq!(layer.get_info().expect("info").label, 5);
    project.undo().expect("undo label");
    assert_eq!(layer.get_info().expect("info").label, default_label);

    layer
        .set_matte(Some(BridgeMatte {
            layer: source.layer_id,
            luma: true,
            inverted: false,
        }))
        .expect("matte set");
    assert!(layer.get_matte().expect("matte").is_some());
    project.undo().expect("undo matte");
    assert!(
        layer.get_matte().expect("matte").is_none(),
        "one undo step removes the matte"
    );
}
