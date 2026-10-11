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
        list_effects, BridgeEffectInstance, BridgeEffectValue, BridgeKeyframe, BridgeRational,
        BridgeScalar, BridgeSideInterp, BridgeUnit,
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
        sequence: None,
        id: Uuid::now_v7(),
        name: "filed.mp4".into(),
        media: MediaRef {
            relative_path: "filed.mp4".into(),
            absolute_path: String::new(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
        source_layer: None,
    };
    let loose = FootageItem {
        sequence: None,
        id: Uuid::now_v7(),
        name: "loose.mp4".into(),
        media: MediaRef {
            relative_path: "loose.mp4".into(),
            absolute_path: String::new(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
        source_layer: None,
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

/// The other direction: filing a loose item into a folder, which is what the
/// panel's drag onto a folder row and its **Move to folder** menu do. One undo
/// step, and an unknown folder is a calm error rather than a panic.
#[test]
fn move_to_folder_files_the_item_and_refuses_an_unknown_folder() {
    let (project, folder, _filed, loose) = project_with_folder();
    let ItemReference::Folder(folder_ref) = &folder else {
        panic!("the fixture built a folder");
    };

    loose.move_to_folder(folder_ref.id()).expect("filed");
    let children = folder_ref.get_children().expect("children");
    assert_eq!(
        children.len(),
        2,
        "the item it held, plus the one just filed"
    );
    assert!(
        children[1].equals(&loose),
        "a filed item lands at the end of the folder"
    );
    assert_eq!(
        project.get_items().expect("roots").len(),
        1,
        "just the folder is left at the root"
    );

    // One undo step, whole.
    project.undo().expect("undone");
    assert_eq!(project.get_items().expect("roots").len(), 2);

    // A folder that is not there, and an id that is not a folder: calm errors.
    assert!(matches!(
        loose.move_to_folder(uuid::Uuid::now_v7()),
        Err(BridgeError::InvalidItem)
    ));
    assert!(matches!(
        loose.move_to_folder(loose.item_id()),
        Err(BridgeError::InvalidItem)
    ));

    // A folder into itself is the cycle refusal, and it says so in its own words.
    assert!(matches!(
        folder.move_to_folder(folder_ref.id()),
        Err(BridgeError::FolderCycle)
    ));
}

/// **Relinking one clip deep inside a moved tree brings the rest of the tree
/// with it.**
///
/// The shape this exists for is an edit's footage: forty-eight clips in
/// forty-eight different subfolders under one root, and the root is what moved.
/// Matching siblings by file name in the folder the user picked finds none of
/// them, because none of them is in that folder. What the move actually says is
/// a prefix rewrite — the tail the two paths share did not move, everything in
/// front of it did — and applying that to every other lost item is one gesture
/// instead of forty-eight.
#[test]
fn relinking_one_clip_rewrites_the_prefix_for_every_other_lost_clip() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    for (folder, file) in [("Cine1", "Depth.avi"), ("Cine5", "World.avi")] {
        std::fs::create_dir_all(root.join("Clips").join(folder)).expect("tree");
        std::fs::write(root.join("Clips").join(folder).join(file), b"clip").expect("clip");
    }

    // Where the project says they were: another root entirely, as an import
    // from another machine leaves them.
    let old_root = std::path::Path::new("/nowhere/Set Me Free Edit");
    let footage = |name: &str, folder: &str| FootageItem {
        colour_space: None,
        sequence: None,
        id: Uuid::now_v7(),
        name: name.into(),
        media: MediaRef {
            relative_path: name.into(),
            absolute_path: old_root
                .join("Clips")
                .join(folder)
                .join(name)
                .to_string_lossy()
                .into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        source_layer: None,
    };
    let picked_item = footage("Depth.avi", "Cine1");
    let sibling = footage("World.avi", "Cine5");
    let (picked_id, sibling_id) = (picked_item.id, sibling.id);

    let project = LumitBridgeState::new_project(None).expect("a new project");
    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        for (index, item) in [
            ProjectItem::Footage(picked_item),
            ProjectItem::Footage(sibling),
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

    let new_path = root.join("Clips").join("Cine1").join("Depth.avi");
    FootageReference::new(project.id, picked_id)
        .relink(new_path.to_string_lossy().into_owned())
        .expect("relinked");

    let state = project.state().expect("state");
    let state = state.read().expect("read");
    let doc = state.store.snapshot();
    let media_of = |id: Uuid| match doc.item(id) {
        Some(ProjectItem::Footage(f)) => f.media.absolute_path.clone(),
        _ => panic!("the footage is still there"),
    };
    assert_eq!(media_of(picked_id), new_path.to_string_lossy());
    assert_eq!(
        media_of(sibling_id),
        root.join("Clips")
            .join("Cine5")
            .join("World.avi")
            .to_string_lossy(),
        "the sibling four folders away moved with the root, not with the folder"
    );
}

/// **A packed project keeps its footage when the original goes.**
///
/// Packed, the original deleted, saved again the ordinary way: the next open
/// still reads the footage out of the file, and an unpack writes it back
/// beside the project and leaves the file without it.
#[test]
fn a_packed_project_keeps_its_footage_when_the_original_goes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let clip = dir.path().join("clip.mov");
    std::fs::write(&clip, b"not really a movie").expect("clip");
    let lum = dir.path().join("scene.lum");

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let footage = project
        .import_footage(clip.to_string_lossy().into_owned())
        .expect("imported");
    let packed = project
        .save_packed(lum.to_string_lossy().into_owned(), true, None)
        .expect("packed");
    assert_eq!((packed.packed, packed.left_out), (1, 0));
    assert_eq!(project.pack_state().expect("state").packed, 1);

    // The original goes, and an ordinary save still carries the footage.
    std::fs::remove_file(&clip).expect("deleted");
    project.new_composition("Later".into(), None).expect("comp");
    project.save(String::new()).expect("saved");

    // What an open does with the file.
    let (mut doc, _) = lumit_project::open(&lum).expect("opens");
    crate::packing::restore(&mut doc, &lum, dir.path(), |_| {});
    let Some(ProjectItem::Footage(read_out)) = doc.item(footage.id) else {
        panic!("the footage is still there");
    };
    assert_eq!(
        std::fs::read(&read_out.media.absolute_path).expect("read out of the file"),
        b"not really a movie"
    );

    let unpacked = project.unpack(None).expect("unpacked");
    assert_eq!((unpacked.written, unpacked.kept), (1, 0));
    assert_eq!(
        std::fs::read(dir.path().join("media").join("clip.mov")).expect("written back out"),
        b"not really a movie"
    );
    assert_eq!(project.pack_state().expect("state").packed, 0);
    let (doc, _) = lumit_project::open(&lum).expect("opens");
    assert!(doc.packed.is_empty(), "the file no longer carries it");

    project.close().expect("closed");
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
    comp.add_footage_layer(footage, false, None)
        .expect("placed");

    let layers = comp.get_layers().expect("layers");
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].get_name().expect("name"), "filed.mp4");
}

/// Add a composition straight through the store, since the frb API has no
/// add-composition op yet (that arrives with the Timeline port).
fn add_comp(project: &ProjectReference, name: &str) -> CompositionReference {
    use lumit_core::model::LinearColour;
    use lumit_core::time::{Duration, FrameRate, Rational};

    let comp = lumit_core::model::Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
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
    // The same answer in every build: whether a file is on disk is a question
    // for the filesystem, not for the decoder. Before that, a
    // media-less build called this path Ready.
    assert!(matches!(status, LumitMediaStatus::Missing));
}

/// Importing then reading back is the panel's whole read path, and `new_composition`
/// must file its comp so the tree has something to nest.
/// **A work area cannot leave the composition.** Dragging its start before frame
/// zero used to store a negative in point, and the cache fill's frame numbers —
/// unsigned — turned that into a first frame of eighteen quintillion, which
/// killed the render worker on a `min > max` and left every later frame request
/// failing with a send error. The op clamps, so no caller can store one: the
/// handle simply stops at the edge.
#[test]
fn a_work_area_is_clamped_to_the_composition() {
    use crate::api::layer::BridgeSpan;
    let project = LumitBridgeState::new_project(None).expect("project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");

    let frames = comp.duration_frames().expect("frames");
    let zero = BridgeRational { num: 0, den: 1 };
    let span = |a: i64, b: i64| BridgeSpan {
        in_point: comp.time_of_frame(a).expect("time"),
        out_point: comp.time_of_frame(b).expect("time"),
        start_offset: zero,
    };

    // Before the start, and past the end: both ends come back inside.
    comp.set_work_area(Some(span(-40, frames + 40)))
        .expect("set");
    let held = comp.get_work_area().expect("read").expect("a span");
    assert_eq!(
        comp.frame_at_time(held.in_point).expect("frame"),
        0,
        "the in point is clamped to the first frame"
    );
    assert_eq!(
        comp.frame_at_time(held.out_point).expect("frame"),
        frames,
        "and the out point to the end of the comp"
    );

    // A span entirely outside has nothing left after clamping, so it is refused
    // rather than stored as an empty one — and the previous span stands.
    assert!(comp.set_work_area(Some(span(-80, -40))).is_err());
    let still = comp.get_work_area().expect("read").expect("a span");
    assert_eq!(comp.frame_at_time(still.in_point).expect("frame"), 0);
}

/// **A folder of numbered stills is one item, not two thousand**.
///
/// The front door's whole promise: pick any file of a run and the run comes in,
/// named for its span, pointed at its first frame — and picking the rest of the
/// files afterwards (which is what selecting the whole folder does) hands back
/// the item that is already there rather than filing it again.
// A build with no decoder imports the picked still as a still, so there is
// no run for this row to ask about.
#[cfg(feature = "media")]
#[test]
fn a_run_of_numbered_stills_imports_once_whichever_file_is_picked() {
    let dir = tempfile::tempdir().expect("temp dir");
    for n in 1..=5u32 {
        std::fs::write(
            dir.path().join(format!("frame{n:04}.png")),
            b"not really a png",
        )
        .expect("write");
    }

    let project = LumitBridgeState::new_project(None).expect("project");
    let first = project
        .import_footage(
            dir.path()
                .join("frame0003.png")
                .to_string_lossy()
                .into_owned(),
        )
        .expect("imported");
    let roots = project.get_items().expect("roots");
    assert_eq!(
        roots.first().map(|i| i.name().expect("name")),
        Some("frame[0001-0005].png".to_owned()),
        "the panel says what the run is and where it stops"
    );

    for n in 1..=5u32 {
        let again = project
            .import_footage(
                dir.path()
                    .join(format!("frame{n:04}.png"))
                    .to_string_lossy()
                    .into_owned(),
            )
            .expect("imported");
        assert_eq!(again.id(), first.id(), "file {n} is the same item");
    }
    assert_eq!(project.get_items().expect("roots").len(), 1);
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
        background: [0.0, 0.0, 0.0, 1.0],
        shutter_angle: 180.0,
        motion_blur_samples: 16,
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

/// **The frame-rate regression.** Changing only the rate must change only
/// the rate: the comp keeps its real length, and a layer keeps the seconds it
/// occupies, so nothing plays faster or slower. Before this, the dialog read the
/// duration as a frame count and wrote the same count back at the new rate, which
/// silently halved or doubled the comp against layers that had not moved.
#[test]
fn changing_only_the_frame_rate_leaves_the_comp_and_its_layers_where_they_were() {
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer(None).expect("layer");
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
            time: lumit_core::time::Rational::new(0, 1).expect("0 s"),
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
            // Deliberately a name no schema declares. This fixture exists to
            // exercise the *value type* — one parameter per kind — not any real
            // effect, and its parameters are invented to match. Borrowing a
            // shipped effect's name would mean `BridgeEffectInstance::new`
            // filling in that effect's own declared parameters beside these
            // which is right for a real instance and pure noise here.
            match_name: "test_every_value_kind".into(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        roto: None,
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
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
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
    layer.set_effects(staged, None).expect("committed");

    assert_eq!(stack_of(&layer), vec![original]);
}

/// The inner shader graph's seam (custom-shader.md §4, §8 item 27, CS4): a
/// staged `set_shader_graph` commits as one `SetLayerEffects` with the compiled
/// text cached beside it; Detach keeps the text and drops the graph — one way,
/// its own undo step; and each step undoes whole.
#[test]
fn a_shader_graph_commits_detaches_one_way_and_undoes_whole() {
    let (project, layer) = project_with_layer();
    seed_stack(
        &project,
        &layer,
        vec![lumit_core::fx::instantiate("custom_shader").expect("the effect exists")],
    );

    let graph = serde_json::json!({
        "nodes": [
            {"id": 1, "kind": "picture"},
            {"id": 2, "kind": "result"},
        ],
        "edges": [{"from": 1, "from_port": 0, "to": 2, "to_port": 0}],
    })
    .to_string();

    // One staged write, one commit, one undo step.
    let mut staged = layer.get_effects().expect("stack");
    staged[0].set_shader_graph(graph).expect("a graph document");
    layer.set_effects(staged, None).expect("committed");

    let held = stack_of(&layer);
    let block = held[0].extra.get("shader").expect("the block");
    assert!(block.get("graph").is_some(), "the graph is stored");
    let cached = block
        .get("source")
        .and_then(|v| v.as_str())
        .expect("the compiled text is cached beside it")
        .to_owned();
    assert!(cached.contains("lumit_sample(uv)"));

    // Detach keeps the text and drops the graph (§4.1) — not reversible by
    // another button, which is the honest shape.
    let mut staged = layer.get_effects().expect("stack");
    assert!(staged[0].shader_graph().is_some());
    staged[0].detach_shader_graph();
    assert!(staged[0].shader_graph().is_none());
    layer.set_effects(staged, None).expect("committed");
    let held = stack_of(&layer);
    let block = held[0].extra.get("shader").expect("the block");
    assert!(
        block.get("graph").is_none(),
        "the graph is gone because the user said so"
    );
    assert_eq!(
        block.get("source").and_then(|v| v.as_str()),
        Some(cached.as_str()),
        "an ordinary hand-written shader is left behind"
    );

    // One undo brings the graph back whole; a second leaves a fresh instance.
    undo_once(&project);
    let held = stack_of(&layer);
    assert!(
        held[0]
            .extra
            .get("shader")
            .and_then(|b| b.get("graph"))
            .is_some(),
        "undoing the detach restores the graph"
    );
    undo_once(&project);
    assert!(
        stack_of(&layer)[0].extra.get("shader").is_none(),
        "undoing the graph edit restores the fresh instance"
    );

    // Nonsense is a caller bug, refused before anything is staged.
    let mut staged = layer.get_effects().expect("stack");
    assert!(staged[0].set_shader_graph("not json".into()).is_err());
}

/// An effect saved before its schema grew a parameter must still be able to
/// reach it.
///
/// `instantiate` copies the schema's parameters at the moment an effect is
/// created, and nothing brings an older instance up to a schema that grew after
/// it. Every such parameter therefore *rendered* right — the resolve step falls
/// back to the declared default — while being **uneditable**, because the read
/// that draws the row and the write behind it both looked only at what the
/// instance already carried. The row came out blank and the write was refused.
///
/// Depth of field is the case that forced this: an instance saved before the
/// aperture folded in could not reach Blades, Roundness, Rotation or
/// Exposure — which is the entire feature.
#[test]
fn an_old_instance_reaches_a_parameter_its_schema_grew_later() {
    let (project, layer) = project_with_layer();
    // A Depth of field as it would have been saved before its aperture controls
    // existed: the schema's instance with those parameters taken back out.
    let mut old = lumit_core::fx::instantiate("dof").expect("dof");
    let grown = [
        "blades",
        "roundness",
        "rotation",
        "exposure",
        "depth_channel",
    ];
    old.params.retain(|p| !grown.contains(&p.id.as_str()));
    // Something the user had already set must survive untouched.
    let radius_before = old
        .params
        .iter()
        .find(|p| p.id == "aperture")
        .expect("aperture")
        .value
        .clone();
    seed_stack(&project, &layer, vec![old]);

    // The read reports every parameter the schema declares, at its default.
    let mut staged = layer.get_effects().expect("stack");
    let info = staged[0].get_info();
    for id in grown {
        assert!(
            info.values.iter().any(|v| v.id == id),
            "{id} must be reported so its row has something to draw"
        );
    }
    assert!(
        matches!(
            staged[0].get_value("depth_channel".into()),
            Ok(BridgeEffectValue::Choice(0))
        ),
        "a grown parameter reads at its declared default (Red)"
    );

    // And the write lands: the parameter is added to the instance rather than
    // refused, and committing keeps it.
    staged[0]
        .set_value(
            "rotation".into(),
            BridgeEffectValue::Float(BridgeScalar::Static(30.0)),
        )
        .expect("a grown parameter must be writable");
    layer.set_effects(staged, None).expect("committed");

    let after = stack_of(&layer);
    let stored = after[0]
        .params
        .iter()
        .find(|p| p.id == "rotation")
        .expect("the written parameter is now on the instance");
    assert!(
        matches!(&stored.value, lumit_core::model::EffectValue::Float(f)
            if (f.value_at(0.0) - 30.0).abs() < 1e-9),
        "the value written is the value stored"
    );
    assert_eq!(
        after[0]
            .params
            .iter()
            .find(|p| p.id == "aperture")
            .expect("aperture")
            .value,
        radius_before,
        "filling absences must never rewrite a value the instance already held"
    );

    // A name no schema declares is still refused — that is a caller bug, not an
    // old project.
    let mut staged = layer.get_effects().expect("stack");
    assert!(
        staged[0]
            .set_value(
                "no_such_param".into(),
                BridgeEffectValue::Float(BridgeScalar::Static(1.0))
            )
            .is_err(),
        "an undeclared parameter is still refused"
    );
}

/// **A hard range is the engine's to keep, not the panel's** (docs/08 §1.2).
///
/// The bounds used to cross the seam as advice and nothing more: every control
/// clamped its own reading, and every path that did not — a keyframe dragged in
/// the graph editor, a number wired from a node, a value picked off the Viewer —
/// wrote straight past them. Worse, it was the *preview* that went out of range
/// while the committed value came back inside it, so a scrub past the end showed
/// a picture the parameter could not hold and then snapped away from it on
/// release.
///
/// So `set_value` clamps, and both the preview and the commit stage through it.
/// A slider's travel is untouched — typing past that is still allowed, which is
/// the whole difference between the two ranges.
#[test]
fn a_value_written_past_a_parameters_hard_range_is_clamped_to_it() {
    use crate::api::effect::{BridgeEffectValue, BridgeRational, BridgeScalar};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let target = comp.add_solid_layer(None).expect("a layer to blur");
    target.add_effect("blur".into()).expect("blur added");

    let radius = |staged: &Vec<crate::api::effect::BridgeEffectInstance>| match staged[0]
        .get_value("radius".into())
    {
        Ok(BridgeEffectValue::Float(BridgeScalar::Static(v))) => v,
        other => panic!("radius reads back as a static float, got {other:?}"),
    };

    // Blur's Radius: slider 0–500, hard 0–2000.
    let mut staged = target.get_effects().expect("stack");
    staged[0]
        .set_value(
            "radius".into(),
            BridgeEffectValue::Float(BridgeScalar::Static(9000.0)),
        )
        .expect("accepted");
    assert!(
        (radius(&staged) - 2000.0).abs() < 1e-9,
        "a value above the hard maximum lands on it, not past it"
    );

    staged[0]
        .set_value(
            "radius".into(),
            BridgeEffectValue::Float(BridgeScalar::Static(-50.0)),
        )
        .expect("accepted");
    assert!(
        radius(&staged).abs() < 1e-9,
        "and a value below the hard minimum lands on that"
    );

    // A number between the slider's end and the hard bound is *not* clamped:
    // typing past the travel is allowed, and only the hard range is not.
    staged[0]
        .set_value(
            "radius".into(),
            BridgeEffectValue::Float(BridgeScalar::Static(900.0)),
        )
        .expect("accepted");
    assert!(
        (radius(&staged) - 900.0).abs() < 1e-9,
        "a slider's travel is a suggestion; typing past it stays legal"
    );

    // **The keys, not only the number under the playhead.** A radius keyed to
    // 9000 two seconds away is as far out of range as one set there now.
    let key = |num: i64, value: f64| crate::api::effect::BridgeKeyframe {
        time: BridgeRational { num, den: 1 },
        value,
        interp_in: crate::api::effect::BridgeSideInterp::Linear,
        interp_out: crate::api::effect::BridgeSideInterp::Linear,
    };
    staged[0]
        .set_value(
            "radius".into(),
            BridgeEffectValue::Float(BridgeScalar::Keyframed(vec![key(0, -10.0), key(2, 9000.0)])),
        )
        .expect("accepted");
    let Ok(BridgeEffectValue::Float(BridgeScalar::Keyframed(keys))) =
        staged[0].get_value("radius".into())
    else {
        panic!("the animated radius reads back keyframed");
    };
    assert_eq!(keys.len(), 2, "clamping moves values, never keys");
    assert!(
        keys[0].value.abs() < 1e-9,
        "the low key came up to the floor"
    );
    assert!(
        (keys[1].value - 2000.0).abs() < 1e-9,
        "and the high key came down to the ceiling"
    );

    // **A one-sided range stays one-sided**. Glow's Threshold clamps at
    // nought below and runs unbounded above — HDR values really do glow harder —
    // so the open end must not acquire a ceiling from the closed one.
    target.add_effect("glow".into()).expect("glow added");
    let mut staged = target.get_effects().expect("stack");
    let glow = staged
        .iter_mut()
        .find(|fx| fx.get_parameters().iter().any(|p| p == "threshold"))
        .expect("the glow is in the stack");
    glow.set_value(
        "threshold".into(),
        BridgeEffectValue::Float(BridgeScalar::Static(120.0)),
    )
    .expect("accepted");
    assert!(
        matches!(
            glow.get_value("threshold".into()),
            Ok(BridgeEffectValue::Float(BridgeScalar::Static(v))) if (v - 120.0).abs() < 1e-9
        ),
        "an unbounded end is unbounded: nothing invents a maximum for it"
    );
    glow.set_value(
        "threshold".into(),
        BridgeEffectValue::Float(BridgeScalar::Static(-3.0)),
    )
    .expect("accepted");
    assert!(
        matches!(
            glow.get_value("threshold".into()),
            Ok(BridgeEffectValue::Float(BridgeScalar::Static(v))) if v.abs() < 1e-9
        ),
        "while the closed end still holds"
    );
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
        layer.set_effects(staged, None),
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

/// Every built-in's parameters survive the crossing. A kind added to the schema
/// without an arm here would panic in the mapping; this walks the lot so that
/// cannot reach a user.
#[test]
fn every_builtin_lists_its_parameters() {
    for info in crate::api::effect::list_effects() {
        // A discovered plugin is in this listing too and its rows are
        // its own; this sweep is a statement about *Lumit's* declarations.
        if info.namespace != crate::api::effect::NAMESPACE_BUILTIN {
            continue;
        }
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

/// The twirls and greying rules cross too, and every rule still names rows the
/// panel will actually be drawing.
///
/// `EffectSchema::groups` existed in the core schema but never crossed the
/// bridge, so Shake and Matte key declared twirls the panel could not know
/// about and drew flat. This is the sweep that keeps the layout side honest
/// now that something depends on it.
#[test]
fn every_builtin_lists_its_layout() {
    for info in crate::api::effect::list_effects() {
        if info.namespace != crate::api::effect::NAMESPACE_BUILTIN {
            continue;
        }
        let groups = crate::api::effect::list_parameter_groups(info.name.clone());
        let enabled_when = crate::api::effect::list_enabled_when(info.name.clone());
        let ids: Vec<String> = crate::api::effect::list_parameters(info.name.clone())
            .into_iter()
            .map(|p| p.id)
            .collect();
        let declared = lumit_core::fx::BUILTINS
            .iter()
            .find(|s| s.match_name == info.name)
            .expect("listed effects are built in");

        assert_eq!(groups.len(), declared.groups.len());
        for g in &groups {
            for member in &g.params {
                assert!(
                    ids.contains(member),
                    "{}: twirl `{}` names `{member}`, which the panel never sees",
                    info.name,
                    g.label
                );
            }
        }
        assert_eq!(enabled_when.len(), declared.enabled_when.len());
        for rule in &enabled_when {
            assert!(ids.contains(&rule.param) && ids.contains(&rule.on));
        }
    }
}

// --- Transform ------------------------------------------------------------

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
    .write_at(&mut group, lumit_core::time::Rational::ZERO)
    .expect("preview write");
    assert_eq!(
        group.opacity.animation,
        lumit_core::anim::Animation::Static(7.0)
    );
}

/// **Separate axes, and back again, is one undo step each way**.
///
/// Separating merges nothing — the axes are already stored apart — so it is a
/// single mode op. Recombining owes the pair a keyframe union, and that union
/// rides in the same batch, so the whole change still undoes at once and puts
/// back both the mode and the keys.
#[test]
fn separating_and_recombining_a_pair_is_one_undo_step_each_way() {
    use crate::api::effect::{BridgeKeyframe, BridgeRational, BridgeScalar, BridgeSideInterp};
    use crate::api::layer::{BridgeAxisMode, BridgeTransformPair, BridgeTransformProp};

    let (project, layer) = project_with_layer();
    let at = |secs: i64| BridgeRational { num: secs, den: 1 };
    let key = |secs: i64, value: f64| BridgeKeyframe {
        time: at(secs),
        value,
        interp_in: BridgeSideInterp::Linear,
        interp_out: BridgeSideInterp::Linear,
    };

    layer
        .set_axis_mode(BridgeTransformPair::Position, BridgeAxisMode::Separated)
        .expect("separated");
    assert_eq!(
        layer.get_info().expect("info").axis_modes.position,
        BridgeAxisMode::Separated
    );

    // A key each, at times the other axis knows nothing about.
    layer
        .set_transform(
            BridgeTransformProp::PositionX,
            BridgeScalar::Keyframed(vec![key(0, 0.0), key(4, 100.0)]),
        )
        .expect("x keyed");
    layer
        .set_transform(
            BridgeTransformProp::PositionY,
            BridgeScalar::Keyframed(vec![key(1, 10.0), key(3, 50.0)]),
        )
        .expect("y keyed");

    layer
        .set_axis_mode(BridgeTransformPair::Position, BridgeAxisMode::Combined)
        .expect("recombined");

    let keys = |scalar: &BridgeScalar| match scalar {
        BridgeScalar::Keyframed(keys) => keys.iter().map(|k| k.time.num).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let after = layer.get_transform().expect("transform");
    assert_eq!(keys(&after.position_x), vec![0, 1, 3, 4]);
    assert_eq!(keys(&after.position_y), vec![0, 1, 3, 4]);

    // One step back: the mode and the merged keys go together, because they
    // were one batch.
    project.undo().expect("undone");
    let back = layer.get_transform().expect("transform");
    assert_eq!(
        layer.get_info().expect("info").axis_modes.position,
        BridgeAxisMode::Separated
    );
    assert_eq!(keys(&back.position_x), vec![0, 4]);
    assert_eq!(keys(&back.position_y), vec![1, 3]);
}

/// **Pan and the master fader round-trip like every other value**, and the
/// fade commands write real keys on the Volume.
#[test]
fn pan_the_master_fader_and_the_fade_commands_round_trip() {
    use crate::api::effect::BridgeScalar;
    use crate::api::layer::BridgeFadeShape;

    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer(None).expect("layer");

    // Pan: centre to start with, one op, one undo step.
    assert!(matches!(
        layer.get_pan().expect("pan"), BridgeScalar::Static(p) if p == 0.0
    ));
    layer.set_pan(BridgeScalar::Static(-50.0)).expect("applied");
    assert!(matches!(
        layer.get_pan().expect("pan"), BridgeScalar::Static(p) if p == -50.0
    ));
    project.undo().expect("undo");
    assert!(matches!(
        layer.get_pan().expect("pan"), BridgeScalar::Static(p) if p == 0.0
    ));

    // The master fader is the comp's, and starts at unity.
    assert_eq!(comp.master_volume_db().expect("master"), 0.0);
    comp.set_master_volume_db(-4.0).expect("applied");
    assert_eq!(comp.master_volume_db().expect("master"), -4.0);
    project.undo().expect("undo");
    assert_eq!(comp.master_volume_db().expect("master"), 0.0);

    // A fade in writes a keyframed Volume — the level at its own moment, and
    // silence at the layer's in point.
    layer.fade_in(0.5, BridgeFadeShape::Ease).expect("faded in");
    assert!(
        matches!(
            layer.get_volume_db().expect("volume"),
            BridgeScalar::Keyframed(_)
        ),
        "a fade is keyframes, not a value"
    );
    // Fading out as well leaves both, and one undo per command.
    layer
        .fade_out(0.5, BridgeFadeShape::Linear)
        .expect("faded out");
    project.undo().expect("undo");
    project.undo().expect("undo");
    assert!(matches!(
        layer.get_volume_db().expect("volume"), BridgeScalar::Static(db) if db == 0.0
    ));

    // A fade with no length is refused rather than writing two keys at once.
    assert!(layer.fade_in(0.0, BridgeFadeShape::Ease).is_err());
    assert!(layer.fade_out(f64::NAN, BridgeFadeShape::Ease).is_err());
}

/// **Detaching audio changes where the sound is edited, never what it is**
/// (owner). The sibling holds the same source over the same span with
/// the same levels, and the original goes quiet — so the mixer's own job list,
/// which is what decides every sample, comes out the same as it was but for
/// which row the sound is filed under. One undo puts the pair back.
///
/// **Needs the decoder.** The row it ends on is a sound-only footage layer, and
/// what makes one sound-only is the probe reading the container and finding no
/// picture in it. With the media feature off nothing can say that, so the
/// layer is an ordinary one and the refusal under test never arrives. The
/// early return below covers a build that has a decoder but cannot read this
/// fixture; this covers a build that has none at all.
#[cfg(feature = "media")]
#[test]
fn detaching_audio_leaves_the_comp_sounding_exactly_as_it_did() {
    let dir = tempfile::tempdir().expect("temp dir");
    let song = dir.path().join("song.wav");
    std::fs::write(&song, silent_wav()).expect("wrote the fixture");

    let project = LumitBridgeState::new_project(None).expect("project");
    let footage = project
        .import_footage(song.to_string_lossy().into_owned())
        .expect("imported");
    let inner = add_comp(&project, "Music");
    inner
        .add_footage_layer(&footage, false, None)
        .expect("the sound is placed");
    if !inner.get_layers().expect("layers")[0]
        .has_audio()
        .expect("asked")
    {
        // No decoder in this build, or none that reads the fixture: there is
        // no sound in the document to detach, so there is no claim to test.
        return;
    }

    // A Precomp layer over that comp: a row with a picture *and* a sound, which
    // is the row this command exists for. (Footage that draws and sings needs a
    // real container; the precomp asks the same question of the same walk.)
    let outer = add_comp(&project, "Edit");
    let precomp = outer.add_precomp_layer(&inner, None).expect("nested");
    precomp
        .set_volume_db(BridgeScalar::Static(-3.0))
        .expect("a level to carry across");

    let before = audible_jobs(&project, &outer);
    assert!(!before.is_empty(), "the song is heard before the detach");

    let sound = precomp.detach_audio().expect("detached");
    assert!(
        audible_jobs(&project, &outer) == before,
        "the same media, span, offset and levels — only the row it sits on moved"
    );

    // Directly below the original, as an Audio layer, with the original muted.
    let rows = outer.get_layers().expect("layers");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].layer_id, sound.layer_id);
    assert_eq!(
        rows[1].get_kind().expect("kind"),
        crate::api::layer::BridgeLayerKind::Audio
    );
    assert_eq!(
        rows[0].get_span().expect("span"),
        rows[1].get_span().expect("span")
    );
    assert!(
        !rows[0].get_switches().expect("switches").audible,
        "the picture row stops sounding, or the clip would be heard twice"
    );

    // One op batch, so one undo takes the whole thing back.
    project.undo().expect("undone");
    let rows = outer.get_layers().expect("layers");
    assert_eq!(rows.len(), 1, "the sibling goes with the mute");
    assert!(rows[0].get_switches().expect("switches").audible);
    assert!(audible_jobs(&project, &outer) == before);

    // **Somebody else's solo does not reach the sibling**. Detaching
    // clones the row, solo and all; a row that was not being heard before the
    // detach must not start being heard because of it, or the command changes
    // the mix it promises to leave alone.
    let mask = outer.add_solid_layer(None).expect("another row");
    mask.set_switch(crate::api::layer::BridgeLayerSwitch::Solo, true)
        .expect("soloed");
    let under_a_solo = audible_jobs(&project, &outer);
    let sound = precomp.detach_audio().expect("detached");
    assert!(
        !sound.get_switches().expect("switches").solo,
        "the sound of a row nobody was hearing stays out of the mix"
    );
    assert!(audible_jobs(&project, &outer) == under_a_solo);
    project.undo().expect("undone");

    // A layer that is already nothing but sound has nothing left to detach
    // from, and says so rather than making a second copy of itself.
    let music_row = inner.get_layers().expect("layers").remove(0);
    assert!(matches!(
        music_row.detach_audio(),
        Err(BridgeError::NoAudio)
    ));
}

/// The comp's audio jobs with the mixer *strip* blanked: the strip is the row
/// the sound is filed under, which is exactly what detaching changes, and
/// everything else on the job is what decides the samples.
///
/// Gated as its callers are: every test that reads jobs is a media test, and
/// a build with the feature off otherwise fails `-D dead_code` on this alone.
#[cfg(feature = "media")]
fn audible_jobs(
    project: &ProjectReference,
    comp: &CompositionReference,
) -> Vec<lumit_render::export::AudioJob> {
    let state = project.state().expect("state");
    let state = state.read().expect("read");
    let doc = state.store.snapshot();
    let composition = doc.comp(comp.id).expect("the comp is there").clone();
    lumit_render::headless::AudioJobsBuilder::new()
        .audio_jobs(&doc, &composition)
        .into_iter()
        .map(|mut job| {
            job.layer = Uuid::nil();
            job
        })
        .collect()
}

/// Sixteen-bit mono PCM, a tenth of a second of silence: enough of a file for
/// a probe to find an audio stream in.
#[cfg(feature = "media")]
fn silent_wav() -> Vec<u8> {
    wav(44_100, &vec![0i16; 4_410])
}

/// Three seconds of click train at 8 kHz: a 30 ms burst every half second, so
/// 120 BPM of beats a detector cannot miss.
fn click_wav() -> Vec<u8> {
    const RATE: u32 = 8_000;
    let mut samples = vec![0i16; RATE as usize * 3];
    for click in 0..6 {
        let start = click * RATE as usize / 2;
        for i in 0..240 {
            samples[start + i] = if i % 2 == 0 { 20_000 } else { -20_000 };
        }
    }
    wav(RATE, &samples)
}

/// Sixteen-bit mono PCM around the samples handed in.
fn wav(rate: u32, samples: &[i16]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
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

    // **The span answers exactly what the singles do** — it exists to move the
    // conversion off the frame (docs/impl/ui-performance.md §4.5), so a span
    // that drifted from `time_of_frame` by one denominator would place
    // keyframes off the frame they were set on (docs/14 §2). Negative frames
    // are in it because they are real: a layer may start before the comp does.
    let span = comp.times_of_frames(-4, 12).expect("span");
    assert_eq!(span.len(), 12);
    for (i, time) in span.iter().enumerate() {
        let frame = -4 + i as i64;
        let single = comp.time_of_frame(frame).expect("time");
        assert_eq!(
            (time.num, time.den),
            (single.num, single.den),
            "the span disagreed with the single call at frame {frame}"
        );
    }

    // And the cap holds, so a frontend asking for a silly span gets a short
    // answer rather than an allocation nobody budgeted for (docs/14).
    assert_eq!(
        comp.times_of_frames(0, u32::MAX).expect("span").len(),
        crate::api::composition::TIME_SPAN_MAX
    );
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
        BridgeLayerKind::NullLayer,
        BridgeLayerKind::Sequence,
        BridgeLayerKind::Light,
    ] {
        let added = match expected {
            BridgeLayerKind::Solid => comp.add_solid_layer(None),
            BridgeLayerKind::Text => comp.add_text_layer(None),
            BridgeLayerKind::Camera => comp.add_camera_layer(None),
            BridgeLayerKind::Adjustment => comp.add_adjustment_layer(None),
            BridgeLayerKind::NullLayer => comp.add_null_layer(None),
            BridgeLayerKind::Sequence => comp.add_sequence_layer(None),
            // The area kind — the one with a size, and so the one
            // worth checking reaches the document intact.
            BridgeLayerKind::Light => comp.add_light_layer(2, None),
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

/// **A copied layer arrives whole, and lands where the playhead is**.
///
/// Copy and paste is the one edit that has to carry *everything* — the transform
/// with its keyframes, the effects with theirs, the switches, the name — because
/// a paste that quietly dropped a property would be found much later, on a shot
/// that looked almost right. So the payload is the document's own `Layer`, and
/// this checks the pieces most likely to be lost by a hand-written conversion.
#[test]
fn a_copied_layer_pastes_whole_and_lands_at_the_playhead() {
    use crate::api::effect::{BridgeEffectValue, BridgeRational, BridgeScalar};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let source = comp.add_solid_layer(None).expect("a layer to copy");
    source.rename("Hero".into()).expect("named");
    source.add_effect("blur".into()).expect("an effect on it");

    // An animated parameter, so the keyframes have something to lose.
    let mut staged = source.get_effects().expect("effects");
    let key = |num: i64, value: f64| crate::api::effect::BridgeKeyframe {
        time: BridgeRational { num, den: 1 },
        value,
        interp_in: crate::api::effect::BridgeSideInterp::Linear,
        interp_out: crate::api::effect::BridgeSideInterp::Linear,
    };
    staged[0]
        .set_value(
            "radius".into(),
            BridgeEffectValue::Float(BridgeScalar::Keyframed(vec![key(0, 0.0), key(2, 40.0)])),
        )
        .expect("animated");
    source.set_effects(staged, None).expect("committed");

    let text = source.copy_layer().expect("copied");

    // Pasted into the same comp at frame 30 (one second at 30 fps).
    let pasted = comp.paste_layer(text.clone(), Some(30)).expect("pasted");
    assert_ne!(
        pasted.layer_id, source.layer_id,
        "a paste is a new layer, not a second name for the old one"
    );
    assert_eq!(pasted.get_name().expect("name"), "Hero", "the name travels");

    let span = pasted.get_span().expect("span");
    assert_eq!(
        (span.in_point.num, span.in_point.den),
        (1, 1),
        "the in point lands on the playhead — frame 30 of a 30 fps comp is 1 s"
    );

    // The effect came too, animated, with an id of its own.
    let fx = pasted.get_effects().expect("effects");
    assert_eq!(fx.len(), 1, "the stack travels");
    assert_ne!(
        fx[0].id(),
        source.get_effects().expect("effects")[0].id(),
        "with a fresh instance id, so no op is ambiguous"
    );
    let Ok(BridgeEffectValue::Float(BridgeScalar::Keyframed(keys))) =
        fx[0].get_value("radius".into())
    else {
        panic!("the animation must survive the round trip");
    };
    assert_eq!(keys.len(), 2, "both keys, unshifted — a layer moves as one");

    // And the original is untouched by any of it.
    assert_eq!(
        source.get_span().expect("span").in_point,
        BridgeRational { num: 0, den: 1 }
    );
}

/// **A pasted effect lands with its first keyframe under the playhead** (the
/// owner's rule). An effect copied from a layer that flashes at 4 s and
/// pasted while the playhead sits at 12 s must flash at 12 s.
#[test]
fn a_pasted_effect_starts_its_animation_at_the_playhead() {
    use crate::api::effect::{BridgeEffectValue, BridgeRational, BridgeScalar};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let source = comp.add_solid_layer(None).expect("a layer");
    source.add_effect("blur".into()).expect("an effect");

    let key = |num: i64, value: f64| crate::api::effect::BridgeKeyframe {
        time: BridgeRational { num, den: 1 },
        value,
        interp_in: crate::api::effect::BridgeSideInterp::Linear,
        interp_out: crate::api::effect::BridgeSideInterp::Linear,
    };
    let mut staged = source.get_effects().expect("effects");
    staged[0]
        .set_value(
            "radius".into(),
            // Two keys a second apart, starting at 4 s.
            BridgeEffectValue::Float(BridgeScalar::Keyframed(vec![key(4, 0.0), key(5, 40.0)])),
        )
        .expect("animated");
    source.set_effects(staged, None).expect("committed");

    let text = source.copy_effects(Vec::new()).expect("copied");
    let target = comp.add_solid_layer(None).expect("somewhere to paste");
    // 12 seconds at 30 fps.
    target.paste_effects(text, 360).expect("pasted");

    let fx = target.get_effects().expect("effects");
    assert_eq!(fx.len(), 1);
    let Ok(BridgeEffectValue::Float(BridgeScalar::Keyframed(keys))) =
        fx[0].get_value("radius".into())
    else {
        panic!("the pasted effect must still be animated");
    };
    assert_eq!(
        keys[0].time,
        BridgeRational { num: 12, den: 1 },
        "the first key sits under the playhead"
    );
    assert_eq!(
        keys[1].time,
        BridgeRational { num: 13, den: 1 },
        "and the rest keep their spacing"
    );
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
        S::AcceptsLights,
        S::Guide,
        // The switch whose write is a batch on a layer born an adjustment —
        // still one undo step, like the ten beside it.
        S::Adjustment,
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
            S::AcceptsLights => start.accepts_lights,
            S::Guide => start.guide,
            S::Adjustment => start.adjustment,
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
    comp.add_footage_layer(&footage, false, None)
        .expect("placed");
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

/// The razor cuts in two without moving anything: a cut that shifted what comes
/// after it would break every edit already in time with the music.
#[test]
fn the_razor_cuts_and_deletes_without_moving_the_other_clips() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false, None)
        .expect("placed");
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

// --- Effect presets -------------------------------------------------------

/// A preset round-trips, and the copy it plants carries fresh instance ids —
/// applying one preset to two layers must not give them effects that share an
/// id, since an id is instance identity and every op that names an effect uses
/// it.
#[test]
fn a_preset_round_trips_with_fresh_instance_ids() {
    let (project, first) = project_with_layer();
    let comp = CompositionReference::new(project.id, first.comp_id());
    first.add_effect("blur".into()).expect("an effect to save");

    let text = first.save_preset("My look".into()).expect("saved");
    assert!(text.contains("\"format\""), "it is a .lumfx document");
    assert!(text.contains("My look"), "and carries its name");

    let second = comp.add_adjustment_layer(None).expect("a second layer");
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

/// A comp nests into another as a Precomp layer, and refuses to nest into
/// itself — the one cycle a user reaches by accident.
#[test]
fn a_composition_nests_into_another_but_not_into_itself() {
    use crate::api::layer::BridgeLayerKind;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let inner = project.new_composition("Inner".into(), None).expect("comp");
    let outer = project.new_composition("Outer".into(), None).expect("comp");

    let placed = outer.add_precomp_layer(&inner, None).expect("nested");
    assert_eq!(placed.get_kind().expect("kind"), BridgeLayerKind::Precomp);
    assert_eq!(placed.get_name().expect("name"), "Inner");

    // The layer points back at the comp it draws, which is what the Hierarchy
    // panel walks.
    let source = placed.get_source_item().expect("source").expect("some");
    assert!(matches!(source, ItemReference::Composition(_)));

    assert!(matches!(
        outer.add_precomp_layer(&outer, None),
        Err(BridgeError::InvalidComp)
    ));
    assert_eq!(outer.get_layers().expect("layers").len(), 1);
}

/// Precompose moving every attribute: the chosen layers go into a new comp as
/// they were, one Precomp layer stands where the topmost of them stood, timing
/// is untouched, and the whole move is one undo step.
#[test]
fn precompose_packs_the_chosen_layers_and_leaves_one_precomp_behind() {
    use crate::api::layer::BridgeLayerKind;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let bottom = comp.add_solid_layer(None).expect("solid");
    let middle = comp.add_solid_layer(None).expect("solid");
    let top = comp.add_solid_layer(None).expect("solid");
    let spans: Vec<_> = [&bottom, &middle, &top]
        .iter()
        .map(|l| l.get_span().expect("span"))
        .collect();

    let packed = comp
        .precompose(
            vec![middle.layer_id, bottom.layer_id],
            String::new(),
            false,
            false,
            None,
        )
        .expect("precomposed");

    // The two go, the untouched one stays, and the new layer takes the deeper
    // pair's place rather than jumping to the front of the stack.
    let after = comp.get_layers().expect("layers");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].layer_id, top.layer_id);
    assert_eq!(after[1].layer_id, packed.layer_id);
    assert_eq!(packed.get_kind().expect("kind"), BridgeLayerKind::Precomp);
    assert_eq!(packed.get_name().expect("name"), "Pre-comp 1");

    // The new comp holds them in stack order, at the times they always had,
    // and is as long as the comp it came out of — so nothing moved in time.
    let Some(ItemReference::Composition(inner)) = packed.get_source_item().expect("source") else {
        panic!("a Precomp layer's source is a composition");
    };
    let inside = inner.get_layers().expect("layers");
    assert_eq!(inside.len(), 2);
    assert_eq!(inside[0].layer_id, middle.layer_id);
    assert_eq!(inside[1].layer_id, bottom.layer_id);
    assert_eq!(inside[0].get_span().expect("span"), spans[1]);
    assert_eq!(inside[1].get_span().expect("span"), spans[0]);
    assert_eq!(
        inner.duration_frames().expect("frames"),
        comp.duration_frames().expect("frames")
    );
    assert_eq!(packed.get_span().expect("span"), spans[2]);

    // One batch, so one undo puts all three layers back where they were.
    project.undo().expect("undo");
    let back = comp.get_layers().expect("layers");
    assert_eq!(back.len(), 3);
    assert_eq!(back[0].layer_id, top.layer_id);
    assert_eq!(back[1].layer_id, middle.layer_id);
    assert_eq!(back[2].layer_id, bottom.layer_id);
}

/// Converting the mix to a precomp, the two-level pack
/// (docs/impl/audio-timeline.md §2, plan 13): a comp per audio row holding one
/// layer per clip, a mix comp holding one Precomp layer per row, one Precomp
/// layer where the topmost row stood, the mark off, and one undo for the lot.
#[test]
fn precompose_sound_mix_nests_a_comp_per_row_and_clears_the_mark() {
    use crate::api::layer::{BridgeFadeShape, BridgeLayerKind, BridgeSpan};
    use lumit_core::model::{Layer, LayerKind};

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/music.wav".into())
        .expect("imported");
    let solid = comp.add_solid_layer(None).expect("solid");

    // A bare Audio layer, trimmed a second in and slid half a second back,
    // faded up with the Audio panel's own command, which is Volume keys.
    comp.add_audio_layer(&footage).expect("a bare Audio layer");
    let plain = comp.get_layers().expect("layers").remove(0);
    plain.rename("Voice".into()).expect("named");
    let span = plain.get_span().expect("span");
    plain
        .set_span(BridgeSpan {
            in_point: BridgeRational { num: 1, den: 1 },
            out_point: span.out_point,
            start_offset: BridgeRational { num: 1, den: 2 },
        })
        .expect("trimmed and slid");
    plain
        .fade_in(0.5, BridgeFadeShape::Ease)
        .expect("faded up from silence");

    // A row of two clips that overlap, so the join is a crossfade nothing
    // but the row's own clip list can see, and a rack of its own.
    comp.add_audio_layer(&footage).expect("one to make a row");
    let row = comp.get_layers().expect("layers").remove(0);
    row.convert_to_sequenced().expect("a row of clips");
    let row = comp.get_layers().expect("layers").remove(0);
    row.rename("Music".into()).expect("named");
    let first = row.get_clips().expect("clips").remove(0);
    row.add_clip(&footage, (first.start_frame + first.end_frame) / 2, true)
        .expect("a second clip, landing on the first");
    // And the first slid past the second, so the row is stored out of time
    // order and the layers have to be put back into it.
    row.slide_clip(first.id, first.end_frame, true)
        .expect("slid past the second");
    row.add_effect("blur".into()).expect("a rack on the row");

    // What the two rows held before the pack, off the document itself.
    let snapshot = || {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot()
    };
    let doc = snapshot();
    let scene = doc.comp(comp.id).expect("the scene");
    let row_before = scene.layers[0].clone();
    let plain_before = scene.layers[1].clone();
    let LayerKind::Sequence {
        clips: clips_before,
    } = row_before.kind.clone()
    else {
        panic!("the row is a list of clips");
    };
    assert_eq!(clips_before.len(), 2);
    assert!(
        clips_before[0].place_start > clips_before[1].place_start,
        "the clips are stored in the order they were made, not in time order"
    );
    assert!(
        matches!(
            plain_before.volume_db.animation,
            lumit_core::anim::Animation::Keyframed(_)
        ),
        "the panel's fade left Volume keys to carry across"
    );

    assert!(!comp.sound_mix().expect("mark"), "a comp opens unmixed");
    comp.set_sound_mix(true).expect("marked");
    let packed = comp.precompose_sound_mix("Mix".into()).expect("packed");

    // The solid stays, and the Precomp stands at the index the topmost row
    // had rather than jumping to the front.
    let after = comp.get_layers().expect("layers");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].layer_id, packed.layer_id);
    assert_eq!(after[1].layer_id, solid.layer_id);
    assert_eq!(
        packed.get_kind().expect("kind"),
        BridgeLayerKind::Audio,
        "the mix holds sound and nothing else, so its layer is an Audio row"
    );
    assert!(!comp.sound_mix().expect("mark"), "and the mark comes off");

    let doc = snapshot();
    let scene = doc.comp(comp.id).expect("the scene");
    let nested = |layer: &Layer| match layer.kind {
        LayerKind::Precomp { comp } => comp,
        _ => panic!("a Precomp layer"),
    };
    let one_clip = |layer: &Layer| match &layer.kind {
        LayerKind::Sequence { clips } if clips.len() == 1 => clips[0].clone(),
        _ => panic!("one clip on a layer of its own"),
    };

    // The mix comp: one Precomp layer per row, in the order the rows stood
    // in, each carrying its row's name, clock, gain, label and switches.
    let mix = doc.comp(nested(&scene.layers[0])).expect("the mix comp");
    assert_eq!(mix.layers.len(), 2);
    assert_eq!(mix.layers[0].name, "Music");
    assert_eq!(mix.layers[1].name, "Voice");
    assert_eq!(mix.layers[1].in_point, plain_before.in_point);
    assert_eq!(mix.layers[1].out_point, plain_before.out_point);
    assert_eq!(mix.layers[1].start_offset, plain_before.start_offset);
    assert_eq!(
        mix.layers[1].volume_db, plain_before.volume_db,
        "the fade rides on the row's own Precomp layer, keys and clock alike"
    );
    assert_eq!(mix.layers[0].switches, row_before.switches);
    assert_eq!(mix.layers[0].label, row_before.label);

    // The row comp: one layer per clip, earliest first, each placed where
    // the clip was placed and each carrying a copy of the row's rack.
    let music = doc.comp(nested(&mix.layers[0])).expect("the row comp");
    assert_eq!(music.layers.len(), 2);
    let early = one_clip(&music.layers[0]);
    let late = one_clip(&music.layers[1]);
    assert!(
        early.place_start < late.place_start,
        "earliest first, because that is the order the mixer reads a row in"
    );
    assert_eq!(early.place_start, clips_before[1].place_start);
    assert_eq!(late.place_start, clips_before[0].place_start);
    assert_eq!(music.layers[0].in_point.0, early.place_start);
    assert_eq!(music.layers[0].out_point.0, early.place_end());
    assert_eq!(music.layers[0].start_offset.0, lumit_core::Rational::ZERO);

    // The crossfade is baked, because a clip alone on a row has no neighbour
    // left to read the join off.
    let overlap = early
        .place_end()
        .checked_sub(late.place_start)
        .expect("the two clips overlap");
    assert!(overlap > lumit_core::Rational::ZERO);
    assert_eq!(late.fade_in.seconds, overlap);
    assert_eq!(early.fade_out.seconds, overlap);

    assert_eq!(row_before.effects.len(), 1);
    for layer in &music.layers {
        assert_eq!(layer.effects.len(), 1, "the rack is on the clips");
        assert_eq!(layer.effects[0].effect, row_before.effects[0].effect);
        assert_ne!(
            layer.effects[0].id, row_before.effects[0].id,
            "an instance is found by its id alone, so no two may share one"
        );
    }
    assert_ne!(music.layers[0].effects[0].id, music.layers[1].effects[0].id);
    for row_layer in &mix.layers {
        assert!(
            row_layer.effects.is_empty(),
            "the rack rides on the clips alone: left on the row's Precomp \
             layer it would be a bus over the whole row (docs/09 §3.1) and \
             would run a second time"
        );
    }

    // A bare Audio layer is one clip, and it goes in the same way.
    let voice = doc
        .comp(nested(&mix.layers[1]))
        .expect("the bare layer's comp");
    assert_eq!(voice.layers.len(), 1);
    let only = one_clip(&voice.layers[0]);
    assert_eq!(only.place_start, only.source_in, "placed where it is read");

    for made in [mix, music, voice] {
        assert!(!made.sound_mix, "there is no road back from a precomp");
    }

    // One undo group, so one step puts the layers and the row back.
    project.undo().expect("undo");
    let back = comp.get_layers().expect("layers");
    assert_eq!(back.len(), 3);
    assert_eq!(back[0].layer_id, row.layer_id);
    assert_eq!(back[1].layer_id, plain.layer_id);
    assert_eq!(back[2].layer_id, solid.layer_id);
    assert!(comp.sound_mix().expect("mark"), "the row is back too");

    // A pack with no name to file is refused, and so is a comp with nothing to
    // pack, rather than either being given an empty comp.
    assert!(matches!(
        comp.precompose_sound_mix("  ".into()),
        Err(BridgeError::EmptyName)
    ));
    let quiet = project.new_composition("Quiet".into(), None).expect("comp");
    quiet.add_solid_layer(None).expect("solid");
    assert!(matches!(
        quiet.precompose_sound_mix("Mix".into()),
        Err(BridgeError::InvalidLayer)
    ));
}

#[cfg(feature = "media")]
#[test]
fn the_packed_mix_plays_the_same_samples() {
    use crate::api::layer::{BridgeFadeShape, BridgeSpan};
    use std::collections::HashMap;
    use std::sync::Arc;

    const RATE: u32 = 8_000;

    let dir = tempfile::tempdir().expect("temp dir");
    let song = dir.path().join("song.wav");
    std::fs::write(&song, click_wav()).expect("wrote the fixture");

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage(song.to_string_lossy().into_owned())
        .expect("imported");
    comp.add_audio_layer(&footage).expect("a bare Audio layer");
    let plain = comp.get_layers().expect("layers").remove(0);
    if !plain.has_audio().expect("asked") {
        // No decoder in this build, or none that reads the fixture: there is
        // no sound in the document to pack, so there is no claim to test.
        return;
    }
    let span = plain.get_span().expect("span");
    plain
        .set_span(BridgeSpan {
            in_point: BridgeRational { num: 1, den: 1 },
            out_point: span.out_point,
            start_offset: BridgeRational { num: 1, den: 2 },
        })
        .expect("trimmed and slid");
    plain
        .fade_in(0.5, BridgeFadeShape::Ease)
        .expect("faded up from silence");

    comp.add_audio_layer(&footage).expect("one to make a row");
    let row = comp.get_layers().expect("layers").remove(0);
    row.convert_to_sequenced().expect("a row of clips");
    let row = comp.get_layers().expect("layers").remove(0);
    let first = row.get_clips().expect("clips").remove(0);
    row.add_clip(&footage, (first.start_frame + first.end_frame) / 2, true)
        .expect("a second clip, landing on the first");

    let duration_s = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        let doc = state.store.snapshot();
        doc.comp(comp.id).expect("the comp").duration.0.to_f64()
    };
    // Every frame of the comp's mix, through the plan the callback plays.
    let mixed = || {
        let jobs = audible_jobs(&project, &comp);
        assert!(!jobs.is_empty(), "the comp has sound in it");
        let mut decoded = HashMap::new();
        for job in &jobs {
            if let std::collections::hash_map::Entry::Vacant(slot) = decoded.entry(job.item) {
                slot.insert(Arc::new(
                    lumit_media::audio::decode_all(&job.path, RATE).expect("decoded"),
                ));
            }
        }
        let (plan, _strips) = crate::audio::build_plan(&jobs, &decoded, RATE, duration_s, 0.0);
        (0..plan.total_frames)
            .map(|i| plan.frame_at(i))
            .collect::<Vec<_>>()
    };

    let before = mixed();
    assert!(
        before.iter().any(|&(l, r)| l != 0.0 || r != 0.0),
        "the fixture is heard, so the comparison means something"
    );

    comp.set_sound_mix(true).expect("marked");
    comp.precompose_sound_mix("Mix".into()).expect("packed");

    assert!(
        mixed() == before,
        "the pack moved the sound about, it did not change it"
    );
}

/// Adjusting the duration trims the new comp to the selection's own span: the
/// packed layer starts at zero inside it, and the Precomp layer covers exactly
/// the stretch the selection covered, so the picture does not move.
#[test]
fn precompose_adjusting_the_duration_trims_the_new_comp_to_the_selection() {
    use crate::api::effect::BridgeRational;
    use crate::api::layer::BridgeSpan;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let solid = comp.add_solid_layer(None).expect("solid");
    // Two seconds of a thirty-second comp, starting at five.
    let span = BridgeSpan {
        in_point: BridgeRational { num: 5, den: 1 },
        out_point: BridgeRational { num: 7, den: 1 },
        start_offset: BridgeRational { num: 5, den: 1 },
    };
    solid.set_span(span).expect("span");

    let packed = comp
        .precompose(vec![solid.layer_id], "Trimmed".into(), false, true, None)
        .expect("precomposed");

    let Some(ItemReference::Composition(inner)) = packed.get_source_item().expect("source") else {
        panic!("a Precomp layer's source is a composition");
    };
    // Two seconds at the comp's rate, not the parent's thirty.
    assert_eq!(
        inner.duration_frames().expect("frames"),
        2 * comp.duration_frames().expect("frames") / 30
    );
    // The packed layer moved back to the start of its new home.
    let inside = inner.get_layers().expect("layers")[0]
        .get_span()
        .expect("span");
    assert_eq!(inside.in_point, BridgeRational { num: 0, den: 1 });
    assert_eq!(inside.out_point, BridgeRational { num: 2, den: 1 });
    assert_eq!(inside.start_offset, BridgeRational { num: 0, den: 1 });
    // And the Precomp layer stands over the moment the selection stood over,
    // with the offset that lines inner time zero up with it.
    assert_eq!(packed.get_span().expect("span"), span);
}

// --- The shell: boot log, tier, autosave and recovery ---------------------

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

/// The export queue is one per process, so two tests queueing at once shift
/// each other's absolute row numbers under the move that is being tested.
/// Every test that queues takes this first.
static EXPORT_QUEUE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Hold the process-wide export queue for the length of a test.
pub(crate) fn export_queue_test() -> std::sync::MutexGuard<'static, ()> {
    EXPORT_QUEUE_TESTS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// The queue holds what it was given and starts nothing on its own.
///
/// Added items wait: *Add to queue* is a list of work, not a start button, and
/// on a machine with no GPU that is also what keeps this test honest — nothing
/// here can launch an encoder. Cancelling a waiting item takes it off the list
/// (it never ran, so it has nothing to report), and every row carries the facts
/// as they were at queue time.
#[test]
fn the_queue_holds_its_items_until_it_is_started() {
    let _queue = export_queue_test();
    use crate::api::export::{
        export_queue_cancel, export_queue_list, export_queue_remove, BridgeExportQueueState,
        BridgeExportSpec,
    };

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let spec = BridgeExportSpec {
        preset: "YouTube 1080p60".into(),
        bitrate_auto: false,
        bitrate_mbps: 16,
        range_start_frame: 4,
        range_end_frame: 12,
        ..BridgeExportSpec::default()
    };

    // Nowhere to write is refused before anything is added.
    assert!(matches!(
        comp.queue_export(spec.clone(), "  ".into(), false),
        Err(BridgeError::NoProjectPath)
    ));

    let first = comp
        .queue_export(spec.clone(), "queued-one.mp4".into(), false)
        .expect("the item is queued");
    let second = comp
        .queue_export(spec, "queued-two.mp4".into(), false)
        .expect("and so is the second");

    let rows = export_queue_list();
    let mine: Vec<_> = rows
        .iter()
        .filter(|row| row.id == first || row.id == second)
        .collect();
    assert_eq!(mine.len(), 2, "both items are in the list");
    let row = mine[0];
    assert_eq!(row.state, BridgeExportQueueState::Waiting);
    assert_eq!(row.preset, "YouTube 1080p60");
    assert_eq!(row.codec, "h264");
    assert_eq!(row.range_start_frame, 4);
    assert_eq!(row.range_end_frame, 12);
    assert!(row.path.ends_with("queued-one.mp4"));
    assert!(!row.comp_name.is_empty(), "the comp's name at queue time");

    // Cancelling a waiting item takes it off the list; removing does the same
    // for one that has already run, and both are safe to repeat.
    export_queue_cancel(first);
    export_queue_remove(second);
    export_queue_remove(second);
    let after = export_queue_list();
    assert!(
        !after.iter().any(|row| row.id == first || row.id == second),
        "a cancelled or removed item leaves the queue"
    );
}

// --- Journalling ----------------------------------------------------------

/// Every commit is written to the crash journal as it happens. Without this the
/// autosave and the recovery dialogue have nothing to recover *from* — the
/// journal is the only record of work done since the last save.
#[test]
fn every_commit_is_journalled_and_a_save_clears_it() {
    let journals = tempfile::tempdir().expect("temp dir");
    let _journals = crate::api::state::journals_in(journals.path());
    let project = LumitBridgeState::new_project(None).expect("a new project");

    let armed = || {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        let handle = state.journal.lock().expect("journal");
        handle.clone()
    };
    let Some(journal) = armed() else {
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
    // From the save on the journal is that file's own. Save As keeps the
    // document's id, and a copy must never be offered its original's edits.
    let unsaved = journal;
    let journal = armed().expect("still journalled");
    assert!(!journal.same_as(&unsaved));

    // An edit after the save is journalled again, or a crash from here would
    // lose everything since the save.
    project
        .new_composition("After".into(), None)
        .expect("an edit");
    assert_eq!(journal.read().expect("journal read").len(), 1);
    // Nothing has closed the project, so this is what a crash leaves behind.
    assert!(journal.ended_badly());

    // Saved under another name, the edits since go with the document, and
    // the file it was is left with nothing to be offered.
    project
        .new_composition("Unsaved".into(), None)
        .expect("an edit");
    let copy = dir.join("copy.lum");
    project
        .save(copy.to_string_lossy().into_owned())
        .expect("saved as");
    assert!(!journal.ended_badly(), "the first file has no journal now");
    assert!(!armed().expect("journalled").same_as(&journal));

    std::fs::remove_dir_all(&dir).ok();
}

/// A project that was never saved leaves nothing behind when it closes. The
/// cache used to keep a journal folder for every project ever made.
#[test]
fn closing_a_never_saved_project_removes_its_journal() {
    let journals = tempfile::tempdir().expect("temp dir");
    let _journals = crate::api::state::journals_in(journals.path());
    let held = || std::fs::read_dir(journals.path()).expect("listed").count();

    let project = LumitBridgeState::new_project(None).expect("a new project");
    project
        .new_composition("Scene".into(), None)
        .expect("an edit");
    assert_eq!(held(), 1, "the edit was journalled");

    project.close().expect("closed");
    assert_eq!(held(), 0, "its journal and folders went with it");
}

/// A project with a file keeps its journal through a close, since recovery
/// can still replay it onto that file.
#[test]
fn closing_a_saved_project_keeps_its_journal() {
    let journals = tempfile::tempdir().expect("temp dir");
    let _journals = crate::api::state::journals_in(journals.path());
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("scene.lum").to_string_lossy().into_owned();

    let project = LumitBridgeState::new_project(None).expect("a new project");
    project.new_composition("Saved".into(), None).expect("comp");
    project.save(target.clone()).expect("saved");
    // Recovery reopens the file, so the edit below lands on what was saved.
    project.restore_journal(target).expect("restored");
    project
        .new_composition("Unsaved".into(), None)
        .expect("an edit");
    assert!(project.ended_badly(), "an open project with unsaved edits");
    let journal = {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        let handle = state.journal.lock().expect("journal");
        handle.clone().expect("a journal")
    };

    project.close().expect("closed");
    assert_eq!(
        std::fs::read_dir(journals.path()).expect("listed").count(),
        1,
        "the unsaved edit is still there to recover"
    );
    assert!(!journal.ended_badly(), "a close on purpose is not a crash");
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

// --- Shape layers ---------------------------------------------------------

use crate::api::layer::BridgeLayerKind;

fn shape_item(name: &str, x: f64, y: f64, side: f64) -> crate::api::layer::BridgeShapeItem {
    use crate::api::layer::{BridgeShapeItem, BridgeVertex};
    let corner = |x: f64, y: f64| BridgeVertex {
        x,
        y,
        tan_in_x: 0.0,
        tan_in_y: 0.0,
        tan_out_x: 0.0,
        tan_out_y: 0.0,
    };
    BridgeShapeItem {
        id: Uuid::now_v7(),
        name: name.into(),
        vertices: vec![
            corner(x, y),
            corner(x + side, y),
            corner(x + side, y + side),
            corner(x, y + side),
        ],
        closed: true,
        fill: Some(crate::api::assets::BridgeColourRgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }),
        stroke: None,
        stroke_width: 0.0,
        opacity: 100.0,
        trim_start: BridgeScalar::Static(0.0),
        trim_end: BridgeScalar::Static(100.0),
        trim_offset: BridgeScalar::Static(0.0),
        dashes: Vec::new(),
        dash_offset: BridgeScalar::Static(0.0),
        gradient: 0,
        gradient_colour: None,
        gradient_start_x: BridgeScalar::Static(0.0),
        gradient_start_y: BridgeScalar::Static(0.0),
        gradient_end_x: BridgeScalar::Static(0.0),
        gradient_end_y: BridgeScalar::Static(0.0),
        combine: 0,
        path_keys: Vec::new(),
        offset_amount: BridgeScalar::Static(0.0),
        repeat_copies: BridgeScalar::Static(1.0),
        repeat_offset: BridgeScalar::Static(0.0),
        repeat_anchor_x: BridgeScalar::Static(0.0),
        repeat_anchor_y: BridgeScalar::Static(0.0),
        repeat_position_x: BridgeScalar::Static(0.0),
        repeat_position_y: BridgeScalar::Static(0.0),
        repeat_rotation: BridgeScalar::Static(0.0),
        repeat_scale: BridgeScalar::Static(100.0),
        repeat_start_opacity: BridgeScalar::Static(100.0),
        repeat_end_opacity: BridgeScalar::Static(100.0),
    }
}

/// A shape tool with nothing selected makes one of these, and it lands where
/// the art was drawn.
#[test]
fn a_shape_layer_is_made_from_its_art_and_placed_where_it_was_drawn() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    let shape = comp
        .add_shape_layer(
            "Rectangle".into(),
            vec![shape_item("Rectangle", 200.0, 100.0, 50.0)],
        )
        .expect("a shape layer");

    assert_eq!(shape.get_kind().expect("kind"), BridgeLayerKind::Shape);
    let contents = shape.get_shape_contents().expect("contents");
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0].name, "Rectangle");
    assert_eq!(contents[0].vertices.len(), 4);

    // Anchored on the art's own corner and positioned at it, so the rectangle
    // is where it was drawn.
    let tf = shape.get_transform().expect("transform");
    let still = |s: &BridgeScalar| match s {
        BridgeScalar::Static(v) => *v,
        _ => panic!("a fresh layer is not keyframed"),
    };
    assert_eq!(still(&tf.anchor_x), 0.0);
    assert_eq!(still(&tf.position_x), 200.0);
    assert_eq!(still(&tf.position_y), 100.0);

    // It is at the top of the stack, where After Effects puts a new shape.
    let layers = comp.get_layers().expect("layers");
    assert_eq!(layers.first().map(|l| l.id()), Some(shape.id()));
}

/// A shape item's Trim start, end and offset round trip, animate, and are
/// clamped to the 0..100 they mean — every key of them.
#[test]
fn a_shapes_trim_round_trips_and_is_clamped() {
    use crate::api::effect::{BridgeKeyframe, BridgeSideInterp};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let shape = comp
        .add_shape_layer("Art".into(), vec![shape_item("Rectangle", 0.0, 0.0, 10.0)])
        .expect("a shape layer");

    let mut contents = shape.get_shape_contents().expect("contents");
    contents[0].trim_start = BridgeScalar::Static(-40.0);
    contents[0].trim_offset = BridgeScalar::Static(180.0);
    contents[0].trim_end = BridgeScalar::Keyframed(vec![
        BridgeKeyframe {
            time: BridgeRational { num: 0, den: 1 },
            value: -10.0,
            interp_in: BridgeSideInterp::Linear,
            interp_out: BridgeSideInterp::Linear,
        },
        BridgeKeyframe {
            time: BridgeRational { num: 1, den: 1 },
            value: 400.0,
            interp_in: BridgeSideInterp::Linear,
            interp_out: BridgeSideInterp::Linear,
        },
    ]);
    shape.set_shape_contents(contents, None).expect("set");

    let got = &shape.get_shape_contents().expect("contents")[0];
    assert_eq!(
        got.trim_start,
        BridgeScalar::Static(0.0),
        "a Start below zero could only ever draw wrongly"
    );
    assert_eq!(
        got.trim_offset,
        BridgeScalar::Static(180.0),
        "the offset is degrees, and degrees wrap"
    );
    let BridgeScalar::Keyframed(keys) = &got.trim_end else {
        panic!("End keeps its keys");
    };
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].value, 0.0);
    assert_eq!(
        keys[1].value, 100.0,
        "and every key is clamped, not just one"
    );
}

/// A shape item's path keys, end to end: the diamond plants a key
/// holding what is already showing, a point drag lands on the key under the
/// playhead rather than on the still path, and the stopwatch off keeps the
/// shape the playhead is over.
#[test]
fn a_shapes_path_keys_hold_what_is_showing_and_take_the_edit_under_the_playhead() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let shape = comp
        .add_shape_layer("Art".into(), vec![shape_item("Rectangle", 0.0, 0.0, 10.0)])
        .expect("a shape layer");
    let id = shape.get_shape_contents().expect("contents")[0].id;
    let at = |num: i64| BridgeRational { num, den: 1 };

    // Unkeyed until somebody keys it, and absent from the read model.
    assert!(shape.get_shape_contents().expect("contents")[0]
        .path_keys
        .is_empty());

    shape.toggle_shape_path_key(id, at(0)).expect("key at 0");
    shape.toggle_shape_path_key(id, at(2)).expect("key at 2");
    let keys = &shape.get_shape_contents().expect("contents")[0].path_keys;
    assert_eq!(keys.len(), 2);
    // Counted up, so the graph draws the rate the shape changes at.
    assert_eq!(keys[0].value, 0.0);
    assert_eq!(keys[1].value, 1.0);

    // A point drag at the second key moves that key's shape, not the first's.
    let mut contents = shape.get_shape_contents().expect("contents");
    contents[0].vertices[0].x = -4.0;
    shape
        .set_shape_contents(contents, Some(at(2)))
        .expect("the drag lands on the key");
    assert_eq!(
        shape.get_shape_contents().expect("contents")[0]
            .path_keys
            .len(),
        2,
        "the drag reused the key there rather than planting a third"
    );

    // Re-timing refuses to step a key over its neighbour, and allows the rest.
    assert!(matches!(
        shape.move_shape_path_key(id, at(2), at(-1)),
        Ok(false)
    ));
    assert!(matches!(
        shape.move_shape_path_key(id, at(2), at(3)),
        Ok(true)
    ));

    // The stopwatch off keeps the shape the playhead is over. At the first key
    // that is the shape as drawn — which is the proof the drag touched only the
    // key it was on.
    shape.clear_shape_path_keys(id, at(0)).expect("clear");
    let after = &shape.get_shape_contents().expect("contents")[0];
    assert!(after.path_keys.is_empty());
    assert_eq!(after.vertices[0].x, 0.0, "the first key never moved");
}

/// An edit that is not a shape edit carries the keys through untouched — an
/// opacity drag must not throw a morph away.
#[test]
fn a_shapes_path_keys_survive_an_edit_that_is_not_a_shape_edit() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let shape = comp
        .add_shape_layer("Art".into(), vec![shape_item("Rectangle", 0.0, 0.0, 10.0)])
        .expect("a shape layer");
    let id = shape.get_shape_contents().expect("contents")[0].id;
    shape
        .toggle_shape_path_key(id, BridgeRational { num: 0, den: 1 })
        .expect("key");

    let mut contents = shape.get_shape_contents().expect("contents");
    contents[0].opacity = 40.0;
    shape.set_shape_contents(contents, None).expect("set");

    let after = &shape.get_shape_contents().expect("contents")[0];
    assert_eq!(after.opacity, 40.0);
    assert_eq!(after.path_keys.len(), 1, "the morph is still there");
}

/// Dragging the left-most point left grows the art's box leftwards, and the
/// layer's origin **is** that box's corner — so without the position following
/// it, every point nobody touched would slide the other way.
#[test]
fn moving_a_point_past_the_arts_edge_leaves_the_rest_of_it_where_it_was() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let shape = comp
        .add_shape_layer(
            "Art".into(),
            vec![shape_item("Rectangle", 200.0, 100.0, 50.0)],
        )
        .expect("a shape layer");

    let still = |s: &BridgeScalar| match s {
        BridgeScalar::Static(v) => *v,
        _ => panic!("not keyframed"),
    };
    // Where an untouched point is drawn: the layer's position plus its offset
    // into the art's box.
    let drawn_at = |index: usize| {
        let contents = shape.get_shape_contents().expect("contents");
        let items: Vec<_> = contents
            .iter()
            .map(|i| {
                i.write_item(lumit_core::time::Rational::ZERO)
                    .expect("an item")
            })
            .collect();
        let (x0, y0, _, _) = lumit_core::shape::contents_bounds(&items, 0.0).expect("a box");
        let tf = shape.get_transform().expect("transform");
        let v = &contents[0].vertices[index];
        (
            still(&tf.position_x) + v.x - x0,
            still(&tf.position_y) + v.y - y0,
        )
    };
    let before = drawn_at(2);

    let mut contents = shape.get_shape_contents().expect("contents");
    contents[0].vertices[0].x -= 30.0;
    contents[0].vertices[0].y -= 20.0;
    shape.set_shape_contents(contents, None).expect("set");

    let tf = shape.get_transform().expect("transform");
    assert_eq!(
        still(&tf.position_x),
        170.0,
        "the layer followed the corner"
    );
    assert_eq!(still(&tf.position_y), 80.0);
    let after = drawn_at(2);
    assert!(
        (after.0 - before.0).abs() < 1e-9 && (after.1 - before.1).abs() < 1e-9,
        "the art nobody dragged stayed where it was: {before:?} became {after:?}"
    );

    project.undo().expect("undone");
    let tf = shape.get_transform().expect("transform");
    assert_eq!(
        (still(&tf.position_x), still(&tf.position_y)),
        (200.0, 100.0),
        "the art and the layer went back together, in one step"
    );
    assert_eq!(
        shape.get_shape_contents().expect("contents")[0].vertices[0].x,
        200.0
    );
}

// --- Puppet: the block and its pins ---------------------------------------

/// Test 15 of docs/impl/puppet.md: a pin placed by *aiming at the picture*
/// round-trips through the document, moves, is deleted, and each of those is one
/// undo step.
///
/// The mesh comes from the render, which is where it is built — so the test
/// leaves one where the render would ([`lumit_render::puppet::publish`]) and
/// then does what the Viewer's click does. That is the whole seam PU3 added: no
/// ghost is a refusal, a point outside it is a refusal, and a point inside it
/// makes the block and the pin together.
#[test]
fn a_pin_is_aimed_at_the_mesh_placed_moved_and_undone() {
    use crate::api::layer::BridgePuppetPinKind;
    let (project, layer) = project_with_layer();

    // No mesh under the click: refused, and no block invented.
    assert!(matches!(
        layer.add_puppet_pin_at(0, BridgePuppetPinKind::Position, "Pin 1".into(), 5.0, 5.0),
        Err(BridgeError::PuppetNoMesh)
    ));
    assert!(layer.get_puppet().expect("puppet").is_none());

    // One triangle, the corner of a layer, deformed eight pixels to the right of
    // where it rests — so a click has a rest position to be carried back to.
    let mesh = std::sync::Arc::new(lumit_core::puppet::PuppetMesh {
        vertices: vec![[0.0, 0.0], [100.0, 0.0], [0.0, 100.0]],
        triangles: vec![[0, 1, 2]],
        hash: [0u8; 32],
    });
    lumit_render::puppet::publish(
        layer.layer_id,
        lumit_render::puppet::Ghost {
            mesh,
            deformed: vec![[8.0, 0.0], [108.0, 0.0], [8.0, 100.0]],
            inert: Vec::new(),
        },
    );

    // Another project closing takes its own wireframes and leaves this one's.
    LumitBridgeState::new_project(None)
        .expect("a second project")
        .close()
        .expect("closed");

    // Outside the deformed triangle: refused, still no block, never a floating
    // pin.
    assert!(matches!(
        layer.add_puppet_pin_at(
            0,
            BridgePuppetPinKind::Position,
            "Pin 1".into(),
            900.0,
            900.0
        ),
        Err(BridgeError::PuppetOutsideMesh)
    ));
    assert!(layer.get_puppet().expect("puppet").is_none());

    // Inside it: the block and the pin land together, and the pin is stored
    // where that spot sits at **rest** — eight pixels left of where it was
    // clicked, which is exactly the deformation.
    let id = layer
        .add_puppet_pin_at(
            0,
            BridgePuppetPinKind::Starch,
            "Shoulder".into(),
            28.0,
            20.0,
        )
        .expect("pinned");
    let read = layer.get_puppet().expect("puppet").expect("a block");
    assert_eq!(read.pins.len(), 1);
    assert_eq!(read.pins[0].id, id);
    assert_eq!(read.pins[0].kind, BridgePuppetPinKind::Starch);
    assert_eq!(
        read.pins[0].x,
        crate::api::effect::BridgeScalar::Static(20.0)
    );
    assert_eq!(
        read.pins[0].y,
        crate::api::effect::BridgeScalar::Static(20.0)
    );

    // The wireframe the overlay draws is that same mesh, flat.
    let ghost = layer.puppet_ghost().expect("ghost").expect("published");
    assert_eq!(ghost.triangles, vec![0, 1, 2]);
    assert_eq!(ghost.vertices, vec![8.0, 0.0, 108.0, 0.0, 8.0, 100.0]);

    // A drag, then a delete, then back a step at a time.
    let mut moved = read.pins[0].clone();
    moved.x = crate::api::effect::BridgeScalar::Static(60.0);
    layer.set_puppet_pin(moved).expect("moved");
    layer.delete_puppet_pin(id).expect("deleted");
    assert!(layer
        .get_puppet()
        .expect("puppet")
        .expect("a block")
        .pins
        .is_empty());

    project.undo().expect("the delete");
    let read = layer.get_puppet().expect("puppet").expect("a block");
    assert_eq!(read.pins.len(), 1);
    assert_eq!(
        read.pins[0].x,
        crate::api::effect::BridgeScalar::Static(60.0)
    );

    project.undo().expect("the drag");
    let read = layer.get_puppet().expect("puppet").expect("a block");
    assert_eq!(
        read.pins[0].x,
        crate::api::effect::BridgeScalar::Static(20.0)
    );

    // And the first pin's undo takes the block it made with it.
    project.undo().expect("the first pin");
    assert!(layer.get_puppet().expect("puppet").is_none());
}

// --- Paint: strokes on a layer --------------------------------------------

fn stroke(name: &str, points: &[(f64, f64)]) -> crate::api::layer::BridgeStroke {
    use crate::api::layer::{BridgePaintMode, BridgeStroke, BridgeStrokePoint};
    BridgeStroke {
        id: Uuid::now_v7(),
        name: name.into(),
        points: points
            .iter()
            .map(|&(x, y)| BridgeStrokePoint {
                x,
                y,
                pressure: 1.0,
            })
            .collect(),
        colour: crate::api::assets::BridgeColourRgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        width: 12.0,
        hardness: 0.8,
        shape: crate::api::layer::BridgeBrushShape::Round,
        opacity: 100.0,
        start: crate::api::effect::BridgeScalar::Static(0.0),
        end: crate::api::effect::BridgeScalar::Static(100.0),
        mode: BridgePaintMode::Paint,
        blend: 0,
        clone_offset_x: 0.0,
        clone_offset_y: 0.0,
    }
}

/// A brush drag is one stroke, one op and one undo step — which is what
/// `Ctrl+Z` after painting has to mean.
#[test]
fn a_stroke_is_added_read_back_and_undone_in_one_step() {
    let (project, layer) = project_with_layer();
    assert!(layer.get_paint().expect("paint").is_empty());

    layer
        .add_stroke(stroke("Brush 1", &[(10.0, 10.0), (40.0, 25.0)]))
        .expect("added");
    let strokes = layer.get_paint().expect("paint");
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].name, "Brush 1");
    assert_eq!(strokes[0].points.len(), 2);
    assert_eq!(strokes[0].points[1].x, 40.0);
    assert_eq!(strokes[0].width, 12.0);

    project.undo().expect("undone");
    assert!(
        layer.get_paint().expect("paint").is_empty(),
        "one stroke, one undo step"
    );
    project.redo().expect("redone");
    assert_eq!(layer.get_paint().expect("paint").len(), 1);
}

// --- Assets: what a layer is made of --------------------------------------

/// **Adding the first animator moves the anchor with it, in one undo step**.
/// An animated line is drawn into a box one text size larger a side with the
/// words that far in, so without the compensating shift the words would jump
/// the moment the first animator arrived — and the shift has to
/// ride in the same `Op` as the document, or `Ctrl+Z` puts the pivot back
/// before it takes the animator away.
#[test]
fn adding_the_first_animator_moves_the_anchor_and_undoes_in_one_step() {
    use crate::api::assets::{
        BridgeColourRgba, BridgeRangeSelector, BridgeSelectorBasis, BridgeSelectorShape,
        BridgeTextAnimator, BridgeTextDocument,
    };
    use crate::api::effect::BridgeScalar;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer(None).expect("a text layer");
    let zero = || BridgeScalar::Static(0.0);
    let plain = BridgeTextDocument {
        text: "Lumit".into(),
        expression: None,
        size: 72.0,
        fill: BridgeColourRgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        path: None,
        path_offset: zero(),
        animators: Vec::new(),
        style: crate::api::assets::default_text_style(),
        paragraph: crate::api::assets::default_paragraph_style(),
    };
    text.set_text(plain.clone()).expect("set");
    let anchor = |t: &crate::api::layer::LayerReference| match t
        .get_transform()
        .expect("transform")
        .anchor_x
    {
        BridgeScalar::Static(v) => v,
        other => panic!("the anchor is not still: {other:?}"),
    };
    let before = anchor(&text);

    let animator = BridgeTextAnimator {
        name: "Cascade".into(),
        selector: BridgeRangeSelector {
            start: zero(),
            end: BridgeScalar::Static(25.0),
            offset: zero(),
            basis: BridgeSelectorBasis::Characters,
            shape: BridgeSelectorShape::Square,
        },
        position_x: zero(),
        position_y: BridgeScalar::Static(-40.0),
        rotation: zero(),
        scale_x: BridgeScalar::Static(100.0),
        scale_y: BridgeScalar::Static(100.0),
        opacity: BridgeScalar::Static(100.0),
        fill_r: zero(),
        fill_g: zero(),
        fill_b: zero(),
    };
    let animated = BridgeTextDocument {
        animators: vec![animator.clone()],
        ..plain.clone()
    };
    text.set_text(animated).expect("set");
    assert!(
        (anchor(&text) - before - 72.0).abs() < 1e-6,
        "the anchor did not follow the margin ({} → {})",
        before,
        anchor(&text)
    );
    let back = text.get_text().expect("text").expect("still text");
    assert_eq!(
        back.animators,
        vec![animator],
        "the animator did not survive"
    );

    // One undo step takes the animator and the pivot away together.
    project.undo().expect("undone");
    assert!(
        text.get_text()
            .expect("text")
            .expect("still text")
            .animators
            .is_empty(),
        "the animator outlived its undo"
    );
    assert!(
        (anchor(&text) - before).abs() < 1e-6,
        "the pivot was left where the animator put it"
    );

    // And editing an animator that is already there moves nothing.
    let with = BridgeTextDocument {
        animators: vec![BridgeTextAnimator {
            name: "Cascade".into(),
            ..back.animators[0].clone()
        }],
        ..plain
    };
    text.set_text(with.clone()).expect("set");
    let settled = anchor(&text);
    text.set_text(BridgeTextDocument {
        animators: vec![BridgeTextAnimator {
            rotation: BridgeScalar::Static(30.0),
            ..with.animators[0].clone()
        }],
        ..with
    })
    .expect("set");
    assert!(
        (anchor(&text) - settled).abs() < 1e-6,
        "editing an animator moved the layer"
    );
}

/// **Text to shapes and Text to points**: each makes a copy beside the
/// original, and the original is untouched — the layer is still a Type layer
/// still saying what it said, which is what makes the commands safe to try.
#[test]
fn converting_a_text_layer_leaves_the_original_where_it_was() {
    use crate::api::assets::{BridgeColourRgba, BridgeTextDocument};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer(None).expect("a text layer");
    text.set_text(BridgeTextDocument {
        text: "Lumit".into(),
        expression: None,
        size: 72.0,
        fill: BridgeColourRgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        path: None,
        path_offset: crate::api::effect::BridgeScalar::Static(0.0),
        animators: Vec::new(),
        style: crate::api::assets::default_text_style(),
        paragraph: crate::api::assets::default_paragraph_style(),
    })
    .expect("set");
    let before = comp.get_layers().expect("layers").len();

    let shapes = text.create_shapes_from_text(0).expect("outlines");
    assert!(
        !shapes.get_shape_contents().expect("contents").is_empty(),
        "the copy has no art"
    );
    assert_eq!(
        text.get_text().expect("text").expect("still text").text,
        "Lumit",
        "the original was converted rather than copied"
    );
    assert_eq!(comp.get_layers().expect("layers").len(), before + 1);

    let points = text.create_points_from_text().expect("points");
    let names: Vec<String> = points
        .get_effects()
        .expect("effects")
        .iter()
        .map(BridgeEffectInstance::name)
        .collect();
    assert!(
        names.iter().any(|n| n == "emit_from_image"),
        "the copy emits nothing: {names:?}"
    );
    assert_eq!(comp.get_layers().expect("layers").len(), before + 2);

    // A line with no ink has no art to make, and says so rather than leaving
    // an empty layer behind.
    text.set_text(BridgeTextDocument {
        text: "   ".into(),
        expression: None,
        size: 72.0,
        fill: BridgeColourRgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        path: None,
        path_offset: crate::api::effect::BridgeScalar::Static(0.0),
        animators: Vec::new(),
        style: crate::api::assets::default_text_style(),
        paragraph: crate::api::assets::default_paragraph_style(),
    })
    .expect("set");
    assert!(matches!(
        text.create_shapes_from_text(0),
        Err(BridgeError::NothingToConvert)
    ));

    // And neither command is offered anything but a Type layer.
    assert!(matches!(
        layer.create_shapes_from_text(0),
        Err(BridgeError::NotText)
    ));
    assert!(matches!(
        layer.create_points_from_text(),
        Err(BridgeError::NotText)
    ));
}

/// A text layer's words are editable and round-trip exactly. Before this the
/// frontend could add a Text layer and never change what it said.
#[test]
fn a_text_layer_round_trips_its_document() {
    use crate::api::assets::{BridgeColourRgba, BridgeTextDocument};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer(None).expect("a text layer");

    let before = text.get_text().expect("text").expect("it is text");
    assert_eq!(before.text, "Text", "the starter document");

    text.set_text(BridgeTextDocument {
        text: "Hello".into(),
        expression: None,
        size: 48.0,
        fill: BridgeColourRgba {
            r: 1.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        },
        path: None,
        path_offset: crate::api::effect::BridgeScalar::Static(0.0),
        animators: Vec::new(),
        style: crate::api::assets::default_text_style(),
        paragraph: crate::api::assets::default_paragraph_style(),
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
            expression: None,
            size: 1.0,
            fill: BridgeColourRgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            },
            path: None,
            path_offset: crate::api::effect::BridgeScalar::Static(0.0),
            animators: Vec::new(),
            style: crate::api::assets::default_text_style(),
            paragraph: crate::api::assets::default_paragraph_style(),
        }),
        Err(BridgeError::NotText)
    ));
}

/// A style and a paragraph round-trip through the document and undo with it.
#[test]
fn a_text_style_round_trips_and_undoes() {
    use crate::api::assets::{BridgeCaps, BridgeKerning, BridgeTextAlign};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer(None).expect("a text layer");
    let plain = text.get_text().expect("text").expect("it is text");
    // New text is the default style with kerning on.
    let mut kerned = crate::api::assets::default_text_style();
    kerned.kerning = BridgeKerning::Metrics;
    assert_eq!(plain.style, kerned);

    let mut styled = plain.clone();
    styled.style.family = "Arial".into();
    styled.style.face = "Bold".into();
    styled.style.leading = Some(90.0);
    styled.style.kerning = BridgeKerning::Off;
    styled.style.tracking = 50.0;
    styled.style.caps = BridgeCaps::Small;
    styled.style.stroke_on = true;
    styled.style.stroke_width = 4.0;
    styled.paragraph.align = BridgeTextAlign::Centre;
    styled.paragraph.space_after = 12.0;
    text.set_text(styled.clone()).expect("set");
    assert_eq!(text.get_text().expect("text").expect("text"), styled);

    project.undo().expect("undone");
    assert_eq!(text.get_text().expect("text").expect("text"), plain);
}

/// **A restyle keeps the words where they are.** A style that changes the
/// box the words are drawn into moves the anchor with it, in the same op, so
/// the first baseline stays put at the side the lines line up on.
#[test]
fn a_restyle_moves_the_anchor_so_the_words_stay_put() {
    use crate::api::assets::{measure_text, BridgeTextAlign};
    use crate::api::effect::BridgeScalar;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer(None).expect("a text layer");
    let anchor = |l: &LayerReference| {
        let t = l.get_transform().expect("transform");
        match (t.anchor_x, t.anchor_y) {
            (BridgeScalar::Static(x), BridgeScalar::Static(y)) => (x, y),
            _ => panic!("a still anchor"),
        }
    };
    let layout = |d: &crate::api::assets::BridgeTextDocument| {
        measure_text(
            d.text.clone(),
            d.size,
            d.style.clone(),
            d.paragraph.clone(),
            false,
        )
    };
    let plain = text.get_text().expect("text").expect("it is text");
    let start = anchor(&text);

    // An outline grows the box round the words, so the baseline moves inside
    // it and the anchor follows by the same amount.
    let mut outlined = plain.clone();
    outlined.style.stroke_on = true;
    outlined.style.stroke_width = 20.0;
    text.set_text(outlined.clone()).expect("set");
    let (was, now) = (layout(&plain), layout(&outlined));
    let moved = anchor(&text);
    assert!(now.left > was.left, "the outline was given room");
    assert!((moved.0 - start.0 - (now.left - was.left)).abs() < 1e-6);
    assert!((moved.1 - start.1 - (now.lines[0].baseline - was.lines[0].baseline)).abs() < 1e-6);

    // One undo puts the style and the anchor back together.
    project.undo().expect("undone");
    assert_eq!(anchor(&text), start);

    // Right-aligned text holds its right edge, so tracking it out moves the
    // anchor by everything the line grew.
    let mut right = plain.clone();
    right.paragraph.align = BridgeTextAlign::Right;
    text.set_text(right.clone()).expect("set");
    let held = anchor(&text);
    let mut tracked = right.clone();
    tracked.style.tracking = 200.0;
    text.set_text(tracked.clone()).expect("set");
    let grown = layout(&tracked).right - layout(&right).right;
    assert!(grown > 1.0, "tracking widened the line");
    assert!((anchor(&text).0 - held.0 - grown).abs() < 1e-6);

    // Retyping the words alone moves nothing.
    let settled = anchor(&text);
    let mut retyped = tracked;
    retyped.text = "Other words".into();
    text.set_text(retyped).expect("set");
    assert_eq!(anchor(&text), settled);
}

/// The seven camera channels ride the transform (docs/impl/camera.md §1), so
/// every row the panel already draws draws them too. A layer that is not a
/// camera carries none of them, and naming one on it is refused.
#[test]
fn a_camera_carries_its_own_channels_and_a_solid_does_not() {
    use crate::api::layer::BridgeTransformProp;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let camera = comp.add_camera_layer(None).expect("a camera");
    let solid = comp.add_solid_layer(None).expect("a solid");

    // The comp is 1920 × 1080, so a fresh camera is the 50 mm lens on it,
    // focused on its own zoom and looking at the middle of the frame.
    let zoom = lumit_core::camera::default_zoom(1920.0);
    let channels = camera
        .get_transform()
        .expect("transform")
        .camera
        .expect("a camera carries its channels");
    assert_eq!(channels.zoom, BridgeScalar::Static(zoom));
    assert_eq!(channels.focus_distance, BridgeScalar::Static(zoom));
    assert_eq!(channels.poi_x, BridgeScalar::Static(960.0));
    assert_eq!(channels.poi_y, BridgeScalar::Static(540.0));
    assert_eq!(channels.poi_z, BridgeScalar::Static(0.0));
    assert!(
        solid.get_transform().expect("transform").camera.is_none(),
        "a solid has no camera channels to carry"
    );

    camera
        .set_transforms(
            vec![BridgeTransformProp::Zoom],
            vec![BridgeScalar::Static(1500.0)],
        )
        .expect("a camera takes its own channel");
    assert_eq!(
        camera
            .get_transform()
            .expect("transform")
            .camera
            .expect("channels")
            .zoom,
        BridgeScalar::Static(1500.0)
    );
    assert!(matches!(
        solid.set_transforms(
            vec![BridgeTransformProp::Zoom],
            vec![BridgeScalar::Static(1500.0)],
        ),
        Err(BridgeError::OpError(
            lumit_core::ops::OpError::PropNotOnLayer
        ))
    ));
}

/// The Front view's wireframes (docs/impl/camera.md §6): a fresh camera's
/// frustum is the comp itself, so its four corners land on the comp's four
/// corners and its eye lands on the middle.
#[test]
fn the_front_view_wireframes_put_a_fresh_cameras_frustum_on_the_comp_corners() {
    use crate::api::layer::{camera_view_pose, BridgeCameraView};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    comp.add_camera_layer(None).expect("a camera");

    let view = camera_view_pose(BridgeCameraView::Front, 1920.0, 1080.0);
    let wires = comp.wireframes(0, view, Vec::new()).expect("wireframes");
    let cam = match wires.cameras.as_slice() {
        [only] => only,
        other => panic!("one camera, not {}", other.len()),
    };
    let close = |got: f64, want: f64| assert!((got - want).abs() < 1.0, "{got} is not {want}");
    for (corner, (x, y)) in
        cam.corners
            .iter()
            .zip([(0.0, 0.0), (1920.0, 0.0), (1920.0, 1080.0), (0.0, 1080.0)])
    {
        assert!(corner.in_front, "the frustum is in front of the Front view");
        close(corner.x, x);
        close(corner.y, y);
    }
    close(cam.eye.x, 960.0);
    close(cam.eye.y, 540.0);
    assert!(
        cam.eye.in_front,
        "the eye sits between the view and the comp"
    );
    assert!(
        cam.point_of_interest.is_none(),
        "a one-node camera has no line to draw"
    );
}

/// Editing a solid changes the **asset**, so every layer drawing it changes at
/// once. That is the point of solids being assets, and the thing a test should
/// pin down before somebody "fixes" it into a per-layer setting.
#[test]
fn editing_a_solid_changes_every_layer_that_uses_it() {
    use crate::api::assets::{BridgeColourRgba, BridgeSolidDef};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    comp.add_solid_layer(None).expect("a solid layer");

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

// --- The sequence view's clip edits ---------------------------------------

/// A Sequence layer built for the clip tests: one clip spanning [0, 4).
#[cfg(test)]
fn sequenced_layer() -> (ProjectReference, CompositionReference, LayerReference) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false, None)
        .expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    layer.convert_to_sequenced().expect("sequenced");
    let layer = comp.get_layers().expect("layers").remove(0);
    (project, comp, layer)
}

/// An **audio row**: an audio-only Sequence layer holding one clip, which is
/// what the Audio timeline draws as a track (docs/impl/audio-timeline.md §2).
/// The footage item comes back too, so a test can drop a second clip on it.
fn audio_row() -> (
    ProjectReference,
    CompositionReference,
    LayerReference,
    FootageReference,
) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/music.wav".into())
        .expect("imported");
    comp.add_audio_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    layer.convert_to_sequenced().expect("a row of clips");
    let layer = comp.get_layers().expect("layers").remove(0);
    (project, comp, layer, footage)
}

/// Re-speeding a clip keeps its place and pins its first frame — the two
/// promises the whole editing surface rests on.
#[test]
fn a_clips_speed_holds_its_place_and_its_first_frame() {
    let (project, _comp, layer) = sequenced_layer();
    let before = layer.get_clips().expect("clips").remove(0);

    layer
        .set_clip_speed(before.id, 200.0, 200.0)
        .expect("re-speeded");
    let after = layer.get_clips().expect("clips").remove(0);

    assert_eq!(after.start_frame, before.start_frame, "the edit point held");
    assert_eq!(after.end_frame, before.end_frame, "and so did its length");
    assert_eq!(after.speed_percent, Some(200.0));
    assert!(after.retimed);

    // One undo step puts it back.
    project.undo().expect("undo");
    let back = layer.get_clips().expect("clips").remove(0);
    assert!(!back.retimed, "un-retimed again, not retimed to 100%");
}

/// Converting keeps the sound: a layer trimmed at the front and slid about
/// converts to a clip covering the same comp frames and reading the same
/// moment of the same file, so the mixer builds the job it built before
/// (docs/impl/audio-timeline.md §2).
#[test]
fn conversion_places_the_clip_at_the_layers_in_point() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/music.wav".into())
        .expect("imported");
    comp.add_audio_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);

    // A non-zero in point and a non-zero start offset, the state a trimmed
    // and slid row is in.
    let span = layer.get_span().expect("span");
    let second = crate::api::effect::BridgeRational { num: 1, den: 1 };
    layer
        .set_span(crate::api::layer::BridgeSpan {
            in_point: second,
            out_point: span.out_point,
            start_offset: crate::api::effect::BridgeRational { num: 1, den: 2 },
        })
        .expect("trimmed and slid");
    let before = layer.get_info().expect("info");

    layer.convert_to_sequenced().expect("sequenced");
    let sequenced = comp.get_layers().expect("layers").remove(0);
    let after = sequenced.get_info().expect("info");
    assert_eq!(
        (after.in_frame, after.out_frame),
        (before.in_frame, before.out_frame),
        "the row covers the frames it covered"
    );
    let clip = after.clips.first().expect("one clip");
    assert_eq!(clip.start_frame, before.in_frame);
    assert_eq!(clip.end_frame, before.out_frame);

    // And it reads the source where the Footage layer read it: half a second
    // in, the distance from the row's own zero to its in point.
    let state = project.state().expect("state");
    let state = state.read().expect("read");
    let doc = state.store.snapshot();
    let Some(lumit_core::model::ProjectItem::Composition(scene)) = doc.item(comp.id) else {
        panic!("the comp");
    };
    let core = scene
        .layers
        .iter()
        .find(|l| l.id == sequenced.id())
        .expect("the row");
    let lumit_core::model::LayerKind::Sequence { clips } = &core.kind else {
        panic!("a sequence row");
    };
    let clip = clips.first().expect("one clip");
    assert_eq!(
        clip.source_in,
        lumit_core::Rational::new(1, 2).expect("½ s")
    );
    assert_eq!(clip.place_start, clip.source_in, "placed where it is read");
}

/// The same row the test above builds, converted: an audio row whose clips do
/// **not** start at the composition's zero, because the row was trimmed a
/// second in and slid half a second back. Every frame the panel hands an edit
/// is a comp frame, so this is the row that catches an edit that forgot the
/// row's own start offset.
fn offset_audio_row() -> (ProjectReference, CompositionReference, LayerReference) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/music.wav".into())
        .expect("imported");
    comp.add_audio_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    let span = layer.get_span().expect("span");
    layer
        .set_span(crate::api::layer::BridgeSpan {
            in_point: crate::api::effect::BridgeRational { num: 1, den: 1 },
            out_point: span.out_point,
            start_offset: crate::api::effect::BridgeRational { num: 1, den: 2 },
        })
        .expect("trimmed and slid");
    layer.convert_to_sequenced().expect("a row of clips");
    let layer = comp.get_layers().expect("layers").remove(0);
    (project, comp, layer)
}

/// A slide on an offset row travels the frames it was asked for, no more: the
/// frame it is given is a comp frame and so is the clip's own start.
#[test]
fn sliding_a_clip_on_an_offset_row_travels_the_frames_asked() {
    let (_project, _comp, layer) = offset_audio_row();
    let before = layer.get_clips().expect("clips").remove(0);
    let length = before.end_frame - before.start_frame;

    layer
        .slide_clip(before.id, before.start_frame + 5, false)
        .expect("slid");
    let after = layer.get_clips().expect("clips").remove(0);
    assert_eq!(after.start_frame, before.start_frame + 5);
    assert_eq!(after.end_frame - after.start_frame, length, "same length");
}

/// A footage item put down on a row: `overlap` keeps the neighbour, so the
/// two clips cross-fade; without it the drop overwrites, which is the picture
/// row's rule (docs/impl/audio-timeline.md §2).
#[test]
fn a_dropped_clip_overlaps_or_overwrites() {
    let (_project, _comp, layer, footage) = audio_row();
    let first = layer.get_clips().expect("clips").remove(0);
    let half = (first.start_frame + first.end_frame) / 2;

    layer.add_clip(&footage, half, true).expect("dropped");
    let kept = layer.get_clips().expect("clips");
    assert_eq!(kept.len(), 2, "the neighbour is still there");
    let landed = kept
        .iter()
        .find(|c| c.id != first.id)
        .expect("the new clip");
    assert_eq!(landed.start_frame, half);
    let held = kept.iter().find(|c| c.id == first.id).expect("the first");
    assert_eq!(held.end_frame, first.end_frame, "and it kept its tail");

    // The same drop without overlap eats what it lands on.
    let (_p2, _c2, plain, footage2) = audio_row();
    let one = plain.get_clips().expect("clips").remove(0);
    let half = (one.start_frame + one.end_frame) / 2;
    plain.add_clip(&footage2, half, false).expect("dropped");
    let after = plain.get_clips().expect("clips");
    let trimmed = after.iter().find(|c| c.id == one.id).expect("the first");
    assert_eq!(trimmed.end_frame, half, "trimmed back to the join");
}

/// A clip moved onto another row is one undo step across two of them, and a
/// move with no target makes a row of its own directly below.
#[test]
fn a_clip_moves_between_rows_in_one_step() {
    use crate::api::layer::BridgeLayerKind;
    let (project, comp, layer, footage) = audio_row();
    let other = comp.add_sequence_layer(None).expect("a second row");
    let clip = layer.get_clips().expect("clips").remove(0);

    layer
        .move_clip(clip.id, Some(other), clip.start_frame + 10, false)
        .expect("moved");
    assert!(
        layer.get_clips().expect("clips").is_empty(),
        "off the row it came from"
    );
    let landed = other.get_clips().expect("clips");
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].start_frame, clip.start_frame + 10);

    project.undo().expect("undo");
    assert_eq!(
        layer.get_clips().expect("clips").len(),
        1,
        "one step puts it back on both rows at once"
    );
    assert!(other.get_clips().expect("clips").is_empty());

    // No target: a new audio-only Sequence layer directly below the source.
    let before = comp.get_layers().expect("layers").len();
    layer
        .move_clip(clip.id, None, clip.start_frame, false)
        .expect("moved to a new row");
    let layers = comp.get_layers().expect("layers");
    assert_eq!(layers.len(), before + 1);
    let made = layers
        .iter()
        .find(|l| !l.equals(&layer) && l.get_clips().is_ok_and(|c| c.len() == 1))
        .expect("the new row");
    assert_eq!(
        made.get_kind().expect("kind"),
        BridgeLayerKind::Audio,
        "an audio-only row"
    );
    let _ = footage;
}

/// A clip's own effect stack is reached by every effect command the layer's
/// stack already has, through the one instance lookup
/// (docs/impl/audio-timeline.md §2).
#[test]
fn a_clips_effects_round_trip_through_the_instance_lookup() {
    let (_project, _comp, layer, _footage) = audio_row();
    let clip = layer.get_clips().expect("clips").remove(0);

    layer
        .add_clip_effect(clip.id, "blur".into())
        .expect("added");
    let stack = layer.get_clip_effects(clip.id).expect("the clip's stack");
    assert_eq!(stack.len(), 1);
    assert!(
        layer.get_effects().expect("the layer's stack").is_empty(),
        "and nothing landed on the layer's own"
    );

    // Bypass and remove both find it by id alone.
    layer
        .set_effect_enabled(&stack[0], false)
        .expect("bypassed");
    let drawn = layer.get_clips().expect("clips").remove(0);
    assert_eq!(drawn.effects.len(), 1, "it rides in on the read model");
    assert!(!drawn.effects[0].enabled);

    layer.remove_effect(&stack[0]).expect("removed");
    assert!(layer.get_clip_effects(clip.id).expect("stack").is_empty());

    // A staged commit of the now-empty clip stack needs the clip named: an
    // empty list carries no id to route by, and without it the write would be
    // held against the layer's own stack instead.
    layer.add_effect("glow".into()).expect("on the layer");
    layer
        .set_effects(Vec::new(), Some(clip.id))
        .expect("the clip's empty stack commits to the clip");
    assert_eq!(
        layer.get_effects().expect("stack").len(),
        1,
        "and the layer's stack is untouched"
    );
    assert!(matches!(
        layer.set_effects(Vec::new(), None),
        Err(BridgeError::StaleEffectStack)
    ));
}

/// Half amplitude, to the precision dB is written in - the level the gain
/// tests below drag a clip to.
const HALF_DB: f64 = -6.020_599_913_279_624;

/// A clip's own gain round-trips and comes back on one undo, like every other
/// clip edit (docs/impl/audio-timeline.md §2), and a level that is not a number
/// is refused rather than written.
#[test]
fn a_clips_gain_round_trips_and_undoes() {
    let (project, _comp, layer, _footage) = audio_row();
    let clip = layer.get_clips().expect("clips").remove(0);
    assert_eq!(clip.gain_db, 0.0, "a fresh clip is at unity");

    layer.set_clip_gain(clip.id, HALF_DB).expect("pulled down");
    assert_eq!(layer.get_clips().expect("clips")[0].gain_db, HALF_DB);

    assert!(
        matches!(
            layer.set_clip_gain(clip.id, f64::NAN),
            Err(BridgeError::InvalidTime)
        ),
        "a level that is not a number saves as null and the project will not reopen"
    );
    assert_eq!(
        layer.get_clips().expect("clips")[0].gain_db,
        HALF_DB,
        "and the refusal left the level where it was"
    );

    project.undo().expect("undo");
    assert_eq!(
        layer.get_clips().expect("clips")[0].gain_db,
        0.0,
        "one step back to unity"
    );
}

/// A Sequence layer converts back to plain footage — the way out of the
/// clip-editing surface, which has to exist because the way in is offered to
/// anyone.
#[test]
fn a_sequence_layer_converts_back_to_footage() {
    let (_project, comp, layer) = sequenced_layer();
    let clip = layer.get_clips().expect("clips").remove(0);
    layer.set_clip_speed(clip.id, 250.0, 250.0).expect("ramped");

    layer.convert_from_sequenced().expect("converted back");
    let back = comp.get_layers().expect("layers").remove(0);
    assert_eq!(back.get_kind().expect("kind"), BridgeLayerKind::Footage);
    // The clip spanned the whole layer, so its map is the layer's map: clip
    // time and layer time were the same clock, and they are the same kind of
    // map, so nothing had to be converted.
    assert!(
        back.get_retime_property().expect("read").is_some(),
        "the ramp came with it"
    );

    // A row of several clips refuses rather than silently losing all but one.
    let (_p2, _c2, many) = sequenced_layer();
    let whole = many.get_info().expect("info");
    many.cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");
    assert!(matches!(
        many.convert_from_sequenced(),
        Err(BridgeError::ManyClips)
    ));
}

/// **A layer's cuts and ramps copy onto another layer**, which is what makes a
/// depth pass follow the footage it belongs to.
#[test]
fn a_sequence_shape_copies_onto_another_layer() {
    let (project, comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    layer
        .cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");
    let first = layer
        .get_clips()
        .expect("clips")
        .into_iter()
        .min_by_key(|c| c.start_frame)
        .expect("the earlier half");
    layer
        .set_clip_speed(first.id, 250.0, 250.0)
        .expect("ramped");
    let shape = layer.copy_sequence_shape(None).expect("copied");

    // A second sequence layer over *different* media, uncut.
    let other_footage = project
        .import_footage("C:/clips/depth.mov".into())
        .expect("imported");
    // Converted rather than auto-wrapped: this path's media does not exist,
    // so the wrap rule correctly declines it (a file it cannot read is not
    // known to run).
    comp.add_footage_layer(&other_footage, false, None)
        .expect("placed");
    comp.get_layers()
        .expect("layers")
        .remove(0)
        .convert_to_sequenced()
        .expect("sequenced");
    let other = comp.get_layers().expect("layers").remove(0);
    assert_eq!(other.get_clips().expect("clips").len(), 1, "one whole clip");
    let source_before = other.get_source_item().expect("item");

    other.paste_sequence_shape(shape).expect("pasted");

    let after = other.get_clips().expect("clips");
    assert_eq!(after.len(), 2, "cut in the same place");
    let earlier = after
        .iter()
        .min_by_key(|c| c.start_frame)
        .expect("the earlier half");
    assert_eq!(
        earlier.speed_percent,
        Some(250.0),
        "and ramped the same way"
    );
    assert_eq!(
        earlier.start_frame, first.start_frame,
        "at the same moment on the comp's clock"
    );
    // The shape carries no media: this layer still plays its own.
    assert!(
        other.get_source_item().expect("item").is_some() == source_before.is_some(),
        "the depth pass is not the footage"
    );
}

/// One ordinary Footage layer in a fresh comp — the un-sequenced twin of
/// [`sequenced_layer`], for the commands that act on a layer's own map.
fn footage_layer() -> (ProjectReference, CompositionReference, LayerReference) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false, None)
        .expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    (project, comp, layer)
}

/// The keys of a layer's Retime map, or a failure saying it has none.
fn retime_keys(layer: &LayerReference) -> Vec<BridgeKeyframe> {
    match layer.get_retime_property().expect("read") {
        Some(BridgeScalar::Keyframed(keys)) => keys,
        other => panic!("expected a keyed map, got {other:?}"),
    }
}

/// **Stretch is sugar over Retime**: half speed makes the layer twice
/// as long, anchored at its in point, and the map that comes out plays the
/// same stretch of source over the longer span.
#[test]
fn a_stretch_halves_the_speed_and_doubles_the_length() {
    let (project, comp, layer) = footage_layer();
    let settings = comp.get_settings().expect("settings");
    let fps = f64::from(settings.fps_num) / f64::from(settings.fps_den);
    let before = layer.get_info().expect("info");
    let span = before.out_frame - before.in_frame;

    layer.stretch(50.0).expect("stretched");

    let after = layer.get_info().expect("info");
    assert_eq!(
        after.in_frame, before.in_frame,
        "the in point is the anchor"
    );
    assert_eq!(
        after.out_frame - after.in_frame,
        span * 2,
        "half speed is twice as long"
    );
    let keys = retime_keys(&layer);
    assert_eq!(keys.len(), 2, "an identity ramp, stretched");
    // The last key still asks for the same source moment: the layer plays the
    // same footage, just over twice the time.
    let last = keys.last().expect("two keys");
    assert!(
        (last.value - (span as f64) / fps).abs() < 1e-6,
        "the same stretch of source, at {}",
        last.value
    );

    // One undo step puts both halves back.
    project.undo().expect("undo");
    let back = layer.get_info().expect("info");
    assert_eq!(back.out_frame, before.out_frame);
    assert!(
        layer.get_retime_property().expect("read").is_none(),
        "and the map went with the length"
    );
}

/// **Freeze at the playhead holds the frame and leaves the layer's length
/// alone** (docs/04 §7.3): a second of hold goes in, everything after
/// it is pushed later, and the tail past the out point is cropped off.
#[test]
fn a_freeze_holds_the_frame_and_keeps_the_length() {
    let (project, comp, layer) = footage_layer();
    let settings = comp.get_settings().expect("settings");
    let fps = f64::from(settings.fps_num) / f64::from(settings.fps_den);
    let before = layer.get_info().expect("info");
    let at = (before.in_frame + before.out_frame) / 2;

    layer.freeze_at_playhead(at).expect("frozen");

    let after = layer.get_info().expect("info");
    assert_eq!(after.in_frame, before.in_frame);
    assert_eq!(after.out_frame, before.out_frame, "the length never moved");

    let keys = retime_keys(&layer);
    // The hold is an ordinary pair of keys a second apart at one value.
    let held = keys
        .windows(2)
        .find(|w| (w[0].value - w[1].value).abs() < 1e-9)
        .expect("a pair holding one moment");
    let seconds = |k: &BridgeKeyframe| k.time.num as f64 / k.time.den as f64;
    assert!(
        (seconds(&held[1]) - seconds(&held[0]) - 1.0).abs() < 1e-6,
        "one second of hold"
    );
    assert!(
        keys.iter()
            .all(|k| seconds(k) * fps <= after.out_frame as f64 + 1e-6),
        "nothing was left past the out point"
    );

    project.undo().expect("undo");
    assert!(
        layer.get_retime_property().expect("read").is_none(),
        "one undo step, and the layer is un-retimed again"
    );
}

/// Trimming an edge brings it in and moves nothing else — no ripple, ever.
#[test]
fn trimming_a_clip_pulls_one_edge_in() {
    let (_project, _comp, layer) = sequenced_layer();
    let before = layer.get_clips().expect("clips").remove(0);

    layer
        .trim_clip(before.id, before.start_frame, before.end_frame - 4)
        .expect("trimmed");
    let after = layer.get_clips().expect("clips").remove(0);
    assert_eq!(after.start_frame, before.start_frame, "the start held");
    assert_eq!(after.end_frame, before.end_frame - 4);

    // And outward again: the map carries on at the speed it was going
    // (docs/04 §7.3), which is what lets a cut clip be lengthened back.
    let out = after.end_frame + 20;
    layer
        .trim_clip(after.id, after.start_frame, out)
        .expect("extended");
    assert_eq!(
        layer.get_clips().expect("clips").remove(0).end_frame,
        out,
        "an edge dragged outward extends rather than snapping back"
    );
}

/// **A clip after a cut keeps starting where it starts.**
///
/// The reported fault: ramping the whole clip was fine, and ramping either
/// half after one cut sent the picture insane — frozen on a frame or two. The
/// map a clip plays by was being *constructed* by the frontend for a clip that
/// had none of its own, and it built it starting at source zero: true only of
/// a clip nobody has cut. Every clip after a cut begins part way into its
/// media, so ramping one threw it back to the top of the file.
///
/// The map now crosses the bridge whether or not the clip has one, built from
/// the clip's real trim-in, so there is nothing to assume.
#[test]
fn a_cut_clips_map_starts_where_the_clip_does() {
    let (_project, _comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    layer
        .cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");

    let clips = layer.get_clips().expect("clips");
    assert_eq!(clips.len(), 2);
    let right = clips
        .iter()
        .max_by_key(|c| c.start_frame)
        .expect("the later half");
    assert!(!right.retimed, "neither half is retimed by a cut");

    // The map it plays by opens on the moment it was cut at, not on zero.
    let BridgeScalar::Keyframed(keys) = &right.retime else {
        panic!("a clip always reports the map it plays by");
    };
    let opens_at = keys.first().expect("a first key").value;
    assert!(
        opens_at > 0.0,
        "the later half starts part way into its media, not at the top of it"
    );

    // And ramping it keeps that: the first frame it shows is the one it showed.
    layer
        .set_clip_speed(right.id, 300.0, 300.0)
        .expect("ramped");
    let ramped = layer
        .get_clips()
        .expect("clips")
        .into_iter()
        .max_by_key(|c| c.start_frame)
        .expect("the later half");
    let BridgeScalar::Keyframed(after) = &ramped.retime else {
        panic!("still a map");
    };
    assert!(
        (after.first().expect("a first key").value - opens_at).abs() < 1e-6,
        "re-speeding pins a clip's first frame, it does not move it          back to the start of the media"
    );
}

// --- Video arriving as a Sequence layer -----------------------------------

/// Placing footage answers with the media's own size and length whether the
/// probe worker got there first or not — the two halves of `crate::probe`,
/// checked through the op that actually needs them.
///
/// The first placement runs with nothing warmed but the import's own request,
/// which may or may not have landed; the second runs with the answer certainly
/// held. Both must produce the same layer, because the fallback probes rather
/// than guessing. Needs an ffmpeg on PATH for the fixture; skips itself
/// without one.
#[test]
#[cfg(feature = "media")]
fn a_placed_layer_is_the_same_whether_the_probe_was_warm_or_not() {
    let dir = tempfile::tempdir().expect("temp dir");
    let Some(clip) = lumit_media::index::tests_support::fixture(dir.path()) else {
        return; // no ffmpeg on this machine
    };

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage(clip.to_string_lossy().into_owned())
        .expect("imported");

    // Cold: whatever the import queued may still be in flight, so this
    // placement is the one that has to stand on the synchronous fallback.
    comp.add_footage_layer(&footage, false, None)
        .expect("placed cold");
    let cold = comp.get_layers().expect("layers")[0]
        .get_info()
        .expect("info");

    // Warm: the answer is certainly held now, so this placement is a look-up.
    let probed = crate::probe::ensure_probed(&clip).expect("the fixture probes");
    assert!(probed.video.is_some(), "the fixture has a picture");
    comp.add_footage_layer(&footage, false, None)
        .expect("placed warm");
    let warm = comp.get_layers().expect("layers")[0]
        .get_info()
        .expect("info");

    assert_eq!(
        cold.out_frame, warm.out_frame,
        "the span comes from the media either way"
    );
    assert!(
        cold.out_frame > 1,
        "the fixture runs for two seconds, so the span is not the one-frame fallback"
    );
}

// --- Retime ------------------------------------------------------------

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
    comp.audio_scrub(12).expect("scrub");
    audio_seek(1.5);
    audio_pause();
    audio_stop();
    assert!(!audio_clock().playing, "stop leaves it stopped");
}

/// **A layer picked as the beat Source is always heard** (owner ruling).
///
/// The comp mix is what is audible — a solo takes everything else out of it,
/// which is the whole point of a solo. Picking a layer by name is a different
/// question: it says *listen to this*, so the named row sounds through a solo
/// on another row and through its own mute. The owner soloed a precomp, ran
/// detection on the music, and was told the composition was silent.
///
/// A soloed picture row over a click train is the smallest scene that tells the
/// two apart: the mix has nothing left in it and says so, while the track asked
/// for by name finds its clicks.
#[test]
fn a_layer_named_as_the_beat_source_is_heard_through_a_solo() {
    use crate::api::beats::BridgeBeatOptions;
    use crate::api::layer::BridgeLayerSwitch;

    let dir = tempfile::tempdir().expect("temp dir");
    let clicks = dir.path().join("clicks.wav");
    std::fs::write(&clicks, click_wav()).expect("wrote the fixture");

    let project = LumitBridgeState::new_project(None).expect("project");
    let comp = add_comp(&project, "Cut");
    let footage = project
        .import_footage(clicks.to_string_lossy().into_owned())
        .expect("imported");
    comp.add_footage_layer(&footage, false, None)
        .expect("placed");
    let music_row = comp.get_layers().expect("layers").remove(0);
    if !music_row.has_audio().expect("asked") {
        // No decoder in this build, or none that reads the fixture: there is
        // nothing to hear either way, so there is no claim to test.
        return;
    }

    // The everyday way to reach the bug: a picture row is soloed, which takes
    // the music out of the mix.
    let solid = comp.add_solid_layer(None).expect("layer");
    solid
        .set_switch(BridgeLayerSwitch::Solo, true)
        .expect("soloed");
    let listening_to = |layer: Option<&crate::api::layer::LayerReference>| BridgeBeatOptions {
        source_layer: layer.map(|l| l.layer_id.to_string()).unwrap_or_default(),
        sensitivity_percent: 80,
        work_area_only: false,
        min_spacing_ms: 200,
        // The clicks are half a second apart, so the grid is named rather than
        // estimated off three seconds of file.
        bpm_override: 120.0,
        phase_ms: 0.0,
    };

    assert!(
        matches!(
            comp.detect_beats(listening_to(None), None),
            Err(BridgeError::NoAudio)
        ),
        "the comp mix hears only what is audible, and the solo silenced it"
    );
    assert!(
        comp.detect_beats(listening_to(Some(&music_row)), None)
            .expect("the named row is heard")
            .placed
            > 0,
        "a layer picked by name is heard through somebody else's solo"
    );

    // And through its own mute, which is the same ruling from the other side.
    music_row
        .set_switch(BridgeLayerSwitch::Audible, false)
        .expect("muted");
    assert!(
        comp.detect_beats(listening_to(Some(&music_row)), None)
            .expect("still heard")
            .placed
            > 0,
        "a layer picked by name is heard with its own mute on"
    );

    // A row that makes no sound at all still says so, named or not.
    assert!(matches!(
        comp.detect_beats(listening_to(Some(&solid)), None),
        Err(BridgeError::NoAudio)
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
        duration_frames: None,
        colour: None,
        is_beat: false,
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
    let read = comp.get_markers().expect("markers");
    assert_eq!(read.len(), 2);
    // The beat band reads the flag off the marker itself: the seeded
    // beat says it is one, and the hand-made cue says it is not.
    assert_eq!(
        read.iter().filter(|m| m.is_beat).count(),
        1,
        "exactly the detected marker crosses as a beat"
    );

    comp.clear_beat_markers().expect("cleared");
    let left = comp.get_markers().expect("markers");
    assert_eq!(left.len(), 1, "the beat went");
    assert_eq!(left[0].label, "Chorus", "and mine stayed");

    // Clearing again is a calm no-op — something a user does without thinking.
    comp.clear_beat_markers().expect("no-op");
    assert_eq!(comp.get_markers().expect("markers").len(), 1);
}

/// **Dragging or renaming a beat marker must leave it a beat marker**.
///
/// The regression: the panel writes the whole list back through `set_markers`,
/// and a bridge marker carries only id, time and label — so every marker was
/// rebuilt with the *default* kind, no duration, and an empty `extra`. Moving a
/// detected beat one frame turned it into an ordinary cue, and *Clear beat
/// markers* then walked straight past it: nothing was left to say it had ever
/// been detected. The ruler's markers made that a drag away.
///
/// The same merge protects a spanning marker's duration and the unknown fields
/// a newer Lumit wrote (docs/10 §1.1), which the panel equally cannot see.
#[test]
fn dragging_a_beat_marker_leaves_it_a_beat_marker() {
    use crate::api::composition::BridgeMarker;
    use crate::api::effect::BridgeRational;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    // A beat marker with a duration and a field from a newer version, placed
    // the way detection and a forward-compatible load place them.
    let beat_id = Uuid::now_v7();
    {
        let mut beat = lumit_core::markers::Marker::beat(
            beat_id,
            lumit_core::Rational::new(1, 1).expect("1 s"),
            0.9,
        );
        beat.duration = Some(lumit_core::Rational::new(1, 4).expect("a quarter second"));
        beat.extra
            .insert("from_a_newer_lumit".into(), serde_json::json!(true));
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(lumit_core::Op::SetCompMarkers {
                comp: comp.id,
                markers: vec![beat],
            })
            .expect("seeded");
    }

    // The panel's write-back: the list it read, with this marker moved and
    // renamed. Read first, exactly as the ruler does — the span crosses now,
    // so a write built from nothing would be the panel saying "make this a
    // moment" rather than the panel not knowing about it.
    let dragged: Vec<BridgeMarker> = comp
        .get_markers()
        .expect("markers")
        .into_iter()
        .map(|m| BridgeMarker {
            time: BridgeRational { num: 3, den: 2 },
            label: "Moved".into(),
            ..m
        })
        .collect();
    comp.set_markers(dragged).expect("dragged");

    let stored = comp.composition().expect("comp").markers;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].label, "Moved", "the edit landed");
    assert_eq!(
        stored[0].time.0,
        lumit_core::Rational::new(3, 2).expect("1.5 s"),
        "and so did the move"
    );
    assert!(
        matches!(
            stored[0].kind,
            lumit_core::markers::MarkerKind::Beat { confidence }
                if (confidence - 0.9).abs() < 1e-6
        ),
        "it is still the beat it was, confidence and all: {:?}",
        stored[0].kind
    );
    assert_eq!(
        stored[0].duration,
        Some(lumit_core::Rational::new(1, 4).expect("a quarter second")),
        "a span nobody resized keeps its exact length, sub-frame and all"
    );
    assert_eq!(
        stored[0].extra.get("from_a_newer_lumit"),
        Some(&serde_json::json!(true)),
        "and the forward-compatibility promise holds across an edit"
    );

    // Which is the whole point: clearing beats still finds it.
    comp.clear_beat_markers().expect("cleared");
    assert!(comp.get_markers().expect("markers").is_empty());
}

/// A composition dropped into another brings its markers with it as the
/// layer's own — **copies**, so editing them never reaches back into the
/// composition they came from, or into anywhere else it is used.
#[test]
fn dropping_a_comp_in_copies_its_markers_onto_the_layer() {
    use crate::api::composition::BridgeMarker;
    use crate::api::effect::BridgeRational;

    let (project, layer) = project_with_layer();
    let outer = CompositionReference::new(project.id, layer.comp_id());
    let source = project
        .new_composition("Beats".into(), None)
        .expect("a comp to drop in");
    let seeded = Uuid::now_v7();
    source
        .set_markers(vec![BridgeMarker {
            duration_frames: None,
            colour: None,
            is_beat: false,
            id: seeded,
            time: BridgeRational { num: 1, den: 2 },
            label: "Drop".into(),
        }])
        .expect("marked");

    let placed = outer.add_precomp_layer(&source, None).expect("placed");
    let on_layer = placed.get_markers().expect("layer markers");
    assert_eq!(on_layer.len(), 1, "the marker came along");
    assert_eq!(on_layer[0].label, "Drop");
    assert_ne!(on_layer[0].id, seeded, "a copy, with an id of its own");

    // Independent from here: clearing the layer's leaves the comp's alone.
    placed.set_markers(vec![]).expect("cleared");
    assert!(placed.get_markers().expect("layer markers").is_empty());
    assert_eq!(
        source.get_markers().expect("comp markers").len(),
        1,
        "the composition it came from is untouched"
    );
}

/// Pre-composing carries the comp's markers into the new comp and leaves the
/// Precomp layer bare: the same cues are on the ruler above it, and drawing
/// them again on the layer would say it twice.
#[test]
fn precompose_carries_markers_in_and_leaves_the_layer_bare() {
    use crate::api::composition::BridgeMarker;
    use crate::api::effect::BridgeRational;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    comp.set_markers(vec![BridgeMarker {
        duration_frames: None,
        colour: None,
        is_beat: false,
        id: Uuid::now_v7(),
        time: BridgeRational { num: 1, den: 2 },
        label: "Chorus".into(),
    }])
    .expect("marked");

    let precomp = comp
        .precompose(vec![layer.layer_id], "Packed".into(), false, false, None)
        .expect("packed");
    assert!(
        precomp.get_markers().expect("layer markers").is_empty(),
        "the Precomp layer draws none of its own"
    );
    assert_eq!(
        comp.get_markers().expect("outer markers").len(),
        1,
        "the outer comp keeps its own"
    );

    let inner = match precomp.get_source_item().expect("source") {
        Some(crate::api::project_item::ItemReference::Composition(c)) => c,
        _ => panic!("a Precomp layer's source is a composition"),
    };
    let packed = inner.get_markers().expect("packed markers");
    assert_eq!(packed.len(), 1, "and the packed comp got a copy");
    assert_eq!(packed[0].label, "Chorus");
    assert_eq!(packed[0].time.num * 2, packed[0].time.den, "still at 0.5 s");
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

    let listed = crate::api::effect::presets_in(dir.path(), "lumfx");
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

/// A preset file reads as preset text whichever kind it is: Lumit's own as it
/// was written, and an After Effects one converted, under its file's name.
#[test]
fn a_preset_file_reads_as_text_whichever_kind_it_is() {
    use crate::api::effect::read_effect_preset;

    let dir = tempfile::tempdir().expect("tempdir");
    let at = |file: &str| dir.path().join(file).to_string_lossy().into_owned();

    let text = r#"{"format":1,"name":"Look","effects":[]}"#;
    std::fs::write(at("look.lumfx"), text).expect("write");
    assert_eq!(
        read_effect_preset(at("look.lumfx")).ok().as_deref(),
        Some(text)
    );

    // The smallest After Effects preset there is: the container, and a
    // description with nothing saved in it.
    std::fs::write(at("Shake.ffx"), b"RIFX\0\0\0\x10FaFXLIST\0\0\0\x04besc").expect("write");
    let converted = read_effect_preset(at("Shake.ffx")).expect("an .ffx converts");
    let preset = lumit_core::preset::from_json(&converted).expect("to a preset");
    assert_eq!(preset.name, "Shake");
    assert!(preset.effects.is_empty());

    assert!(read_effect_preset(at("gone.lumfx")).is_err());
}

// ---------------------------------------------------------------------------
// The keymap (docs/07 §15)
// ---------------------------------------------------------------------------

/// The keymap is one per session by design — there is one window and one set of
/// shortcuts — so every test below edits the *same* map. Cargo runs tests in
/// parallel threads within one process, so without this they would rebind each
/// other's chords and fail a different one each run. Taking it is the price of
/// testing a global, and it is cheaper than pretending the global is not there.
static KEYMAP_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Hold the keymap for the length of a test, starting from the shipped default
/// whatever the previous test left behind.
fn keymap_test() -> std::sync::MutexGuard<'static, ()> {
    let guard = KEYMAP_TESTS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    crate::api::keymap::keymap_load_preset(crate::api::keymap::BridgeKeymapPreset::Lumit);
    guard
}

/// Rebinding is what a row's chord cell does, and the answer it returns is the
/// table to redraw — so the page never has to ask again to show the change.
#[test]
fn rebinding_a_row_answers_with_the_table_it_produced() {
    use crate::api::keymap::*;
    let _guard = keymap_test();

    let after = keymap_rebind(
        BridgeKeyContext::Timeline,
        "layer.duplicate".into(),
        "Mod+Alt+D".into(),
    )
    .expect("a valid chord is taken");
    let row = after
        .iter()
        .flat_map(|g| &g.bindings)
        .find(|b| b.action == "layer.duplicate")
        .expect("the row is still there");
    assert_eq!(row.chord, "Mod+Alt+D");

    // And the dispatch path agrees immediately — one keymap, not two.
    assert_eq!(
        keymap_lookup(BridgeKeyContext::Timeline, "Mod+Alt+D".into()).as_deref(),
        Some("layer.duplicate")
    );
    assert_eq!(
        keymap_lookup(BridgeKeyContext::Timeline, "Mod+D".into()),
        None,
        "the old chord stopped meaning it"
    );
}

/// The handle modifier set in Settings is in the file that gets stored, and a
/// preset puts Alt back.
#[test]
fn the_handle_modifier_is_stored_with_the_keymap_and_reset_by_a_preset() {
    use crate::api::keymap::*;
    let _guard = keymap_test();

    assert_eq!(keymap_break_handles(), BridgeHandleModifier::Alt);
    assert_eq!(
        keymap_set_break_handles(BridgeHandleModifier::Ctrl),
        BridgeHandleModifier::Ctrl
    );

    let json = keymap_to_json();
    keymap_load_preset(BridgeKeymapPreset::Lumit);
    assert_eq!(keymap_break_handles(), BridgeHandleModifier::Alt);
    keymap_from_json(json).expect("the stored file reads back");
    assert_eq!(keymap_break_handles(), BridgeHandleModifier::Ctrl);
}

/// A corrupt stored blob, or somebody else's JSON, leaves the keymap alone
/// rather than half-applying — otherwise one bad file costs every shortcut.
#[test]
fn junk_json_is_refused_whole_and_the_live_keymap_stands() {
    use crate::api::keymap::*;
    let _guard = keymap_test();
    let before = keymap_groups();

    for junk in ["", "{}", "not json at all", r#"{"bindings":[]}"#] {
        let err = keymap_from_json(junk.into()).expect_err("refused");
        assert!(matches!(err, BridgeError::InvalidKeymapFile(_)), "{junk}");
    }
    assert_eq!(keymap_groups(), before, "every shortcut survived the junk");
}

// ---------------------------------------------------------------------------
// The reveal shortcuts (docs/07 §4.3)
// ---------------------------------------------------------------------------

/// `UU` opens what has been *changed*, keyframed or not — the two reveals are
/// different questions, and a moved-but-unkeyed layer is the case that shows it.
#[test]
fn the_modified_reveal_catches_a_change_that_was_never_keyframed() {
    use crate::api::layer::BridgeRevealKind;
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer(None).expect("layer");

    assert!(
        !layer
            .reveal_groups(BridgeRevealKind::Modified)
            .expect("answered")
            .any,
        "a layer nobody has touched reveals nothing"
    );

    layer
        .set_transform(
            crate::api::layer::BridgeTransformProp::Opacity,
            BridgeScalar::Static(50.0),
        )
        .expect("set");

    assert!(
        layer
            .reveal_groups(BridgeRevealKind::Modified)
            .expect("answered")
            .transform,
        "a changed value counts as modified"
    );
    assert!(
        !layer
            .reveal_groups(BridgeRevealKind::Animated)
            .expect("answered")
            .transform,
        "but it is not animated, and U must not claim it is"
    );
}

/// Switching Retime off re-hangs the layer on its source: it keeps its
/// in point, shows the same frame there, and runs at source rate until the
/// source runs out — never longer than it already was.
#[test]
fn switching_retime_off_re_hangs_the_layer_on_its_source() {
    use crate::api::composition::BridgeCompSettings;
    use crate::api::effect::BridgeRational;
    use crate::api::layer::BridgeSpan;

    let rational = |num: i64, den: i64| BridgeRational { num, den };
    let project = LumitBridgeState::new_project(None).expect("a new project");
    // A five-second source at 60 fps: 300 frames of material, and no file to
    // probe — a nested comp's length is its own.
    let inner = project
        .new_composition(
            "Inner".into(),
            Some(BridgeCompSettings {
                name: "Inner".into(),
                width: 320,
                height: 240,
                fps_num: 60,
                fps_den: 1,
                duration: rational(5, 1),
                background: [0.0, 0.0, 0.0, 1.0],
                shutter_angle: 180.0,
                motion_blur_samples: 16,
            }),
        )
        .expect("comp");
    let outer = project.new_composition("Outer".into(), None).expect("comp");
    let layer = outer.add_precomp_layer(&inner, None).expect("nested");

    // Retimed, a layer is any length it likes: stretched to twenty seconds.
    layer.toggle_retime_property().expect("on");
    layer
        .set_span(BridgeSpan {
            in_point: rational(0, 1),
            out_point: rational(20, 1),
            start_offset: rational(0, 1),
        })
        .expect("stretched");

    layer.toggle_retime_property().expect("off");
    let span = layer.get_span().expect("span");
    assert_eq!(
        outer.frame_at_time(span.out_point).expect("frame"),
        300,
        "showing the source's first frame, it runs the source's whole length"
    );
    assert_eq!(
        outer.frame_at_time(span.start_offset).expect("frame"),
        0,
        "and its own zero stays where that frame is"
    );
    assert_eq!(outer.frame_at_time(span.in_point).expect("frame"), 0);

    // Anchored two seconds into the source instead: only the three seconds
    // that are left of the source remain.
    layer.toggle_retime_property().expect("on");
    layer
        .set_span(BridgeSpan {
            in_point: rational(0, 1),
            out_point: rational(20, 1),
            start_offset: rational(-2, 1),
        })
        .expect("stretched");
    layer.toggle_retime_property().expect("off");
    let span = layer.get_span().expect("span");
    assert_eq!(
        outer.frame_at_time(span.out_point).expect("frame"),
        180,
        "three seconds of source were left to play"
    );
    assert_eq!(
        outer.frame_at_time(span.start_offset).expect("frame"),
        -120,
        "the anchor frame still shows at the in point"
    );

    // And it never grows: a layer shorter than what is left keeps its length.
    layer.toggle_retime_property().expect("on");
    layer
        .set_span(BridgeSpan {
            in_point: rational(0, 1),
            out_point: rational(1, 1),
            start_offset: rational(0, 1),
        })
        .expect("trimmed");
    layer.toggle_retime_property().expect("off");
    assert_eq!(
        outer
            .frame_at_time(layer.get_span().expect("span").out_point)
            .expect("frame"),
        60,
        "one second in, one second out"
    );
}

/// A Retime flattened to one constant is a Retime **removed**, not a layer
/// frozen on one frame.
///
/// The reported bug: turning the Retime row's stopwatch off — or deleting the
/// last key, which the graph editor also answers with a static value — wrote a
/// constant map. A constant map says "show this one source moment for the whole
/// layer", so the layer sat on a single frame for ever, with the row gone quiet
/// and nothing on screen to say why. Both gestures mean "no more retime", so
/// both take the Ctrl+Alt+T-off route: the property goes and the layer is
/// re-hung on its source, in one undo step.
#[test]
fn a_flattened_retime_is_removed_rather_than_freezing_the_layer() {
    use crate::api::composition::BridgeCompSettings;
    use crate::api::effect::{BridgeRational, BridgeScalar};

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let inner = project
        .new_composition(
            "Inner".into(),
            Some(BridgeCompSettings {
                name: "Inner".into(),
                width: 320,
                height: 240,
                fps_num: 60,
                fps_den: 1,
                duration: BridgeRational { num: 5, den: 1 },
                background: [0.0, 0.0, 0.0, 1.0],
                shutter_angle: 180.0,
                motion_blur_samples: 16,
            }),
        )
        .expect("comp");
    let outer = project.new_composition("Outer".into(), None).expect("comp");
    let layer = outer.add_precomp_layer(&inner, None).expect("nested");

    layer.toggle_retime_property().expect("on");
    assert!(layer.get_retime_property().expect("read").is_some());

    // The stopwatch turned off: the value the curve read at the playhead,
    // written as a constant.
    layer
        .set_retime_property(BridgeScalar::Static(0.0))
        .expect("de-animated");
    assert!(
        layer.get_retime_property().expect("read").is_none(),
        "a constant map takes the Retime away instead of freezing the layer"
    );

    // And the layer is re-hung on its source, so it plays at source rate again:
    // five seconds of source from the frame that was showing at the in point.
    let span = layer.get_span().expect("span");
    assert_eq!(outer.frame_at_time(span.in_point).expect("frame"), 0);
    assert_eq!(
        outer.frame_at_time(span.out_point).expect("frame"),
        300,
        "the whole source runs again rather than one frame holding"
    );

    // One undo step covers the removal and the re-hang together.
    project.undo().expect("undone");
    assert!(
        layer.get_retime_property().expect("read").is_some(),
        "the Retime comes back whole"
    );

    // A layer with no Retime at all still refuses, rather than being given one.
    layer.toggle_retime_property().expect("off");
    assert!(layer
        .set_retime_property(BridgeScalar::Static(0.0))
        .is_err());
}

/// Keyframes belong to the layer, and the seam says so in the interface's units.
///
/// The engine keys every property in the layer's **own** time, which is what
/// makes a layer's animation travel with it when it is moved. The Timeline
/// draws and edits in **comp** frames. The bridge is where the two meet: what
/// crosses is comp time, converted by the layer's `start_offset` in both
/// directions. Read raw, a moved layer's keys drew at the start of the comp.
#[test]
fn keyframes_cross_on_the_comp_clock_and_travel_with_the_layer() {
    use crate::api::effect::{BridgeKeyframe, BridgeRational, BridgeScalar, BridgeSideInterp};
    use crate::api::layer::{BridgeSpan, BridgeTransformProp};

    let rational = |num: i64, den: i64| BridgeRational { num, den };
    let key = |seconds: i64, value: f64| BridgeKeyframe {
        time: rational(seconds, 1),
        value,
        interp_in: BridgeSideInterp::Linear,
        interp_out: BridgeSideInterp::Linear,
    };

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let layer = comp.add_solid_layer(None).expect("solid");

    // A key at comp second 2, written the way a panel writes one.
    layer
        .set_transform(
            BridgeTransformProp::PositionX,
            BridgeScalar::Keyframed(vec![key(2, 100.0)]),
        )
        .expect("keyed");
    assert_eq!(
        layer.get_transform().expect("transform").position_x,
        BridgeScalar::Keyframed(vec![key(2, 100.0)]),
        "it reads back at the comp time it was written at"
    );

    // Move the whole layer three seconds later — in, out and the offset all
    // shift, which is what a bar drag commits.
    layer
        .set_span(BridgeSpan {
            in_point: rational(3, 1),
            out_point: rational(8, 1),
            start_offset: rational(3, 1),
        })
        .expect("moved");
    assert_eq!(
        layer.get_transform().expect("transform").position_x,
        BridgeScalar::Keyframed(vec![key(5, 100.0)]),
        "the key travelled with the layer, and says so in comp time"
    );
}

/// Switching Retime on keys the layer where it *is*: one key on its in
/// point, one on its out point, both in comp time — not at the start of the
/// composition, and not stopping short of a trimmed layer's tail.
#[test]
fn enabling_retime_keys_the_layer_where_it_sits() {
    use crate::api::effect::{BridgeRational, BridgeScalar};
    use crate::api::layer::BridgeSpan;

    let rational = |num: i64, den: i64| BridgeRational { num, den };
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let inner = project.new_composition("Inner".into(), None).expect("comp");
    let layer = comp.add_precomp_layer(&inner, None).expect("nested");

    // Moved to comp second 3 and trimmed a second off its head: its own zero
    // sits at comp second 2, so local time at the in point is one second.
    layer
        .set_span(BridgeSpan {
            in_point: rational(3, 1),
            out_point: rational(8, 1),
            start_offset: rational(2, 1),
        })
        .expect("placed");

    assert!(layer.toggle_retime_property().expect("on"));
    let Some(BridgeScalar::Keyframed(keys)) = layer.get_retime_property().expect("retime") else {
        panic!("switching Retime on installs a keyed map");
    };
    assert_eq!(keys.len(), 2, "one key on each end, and no others");
    assert_eq!(
        comp.frame_at_time(keys[0].time).expect("frame"),
        comp.frame_at_time(rational(3, 1)).expect("frame"),
        "the first key is on the layer's in point"
    );
    assert_eq!(
        comp.frame_at_time(keys[1].time).expect("frame"),
        comp.frame_at_time(rational(8, 1)).expect("frame"),
        "and the second on its out point"
    );
    // The values are the source times those moments show — the identity map,
    // so each is the layer's own local time and nothing moves on screen.
    assert!((keys[0].value - 1.0).abs() < 1e-9);
    assert!((keys[1].value - 6.0).abs() < 1e-9);
}

/// A mask carries two things `BridgeMask` does not describe — its path
/// keyframes and the forward-compatibility `extra` a newer Lumit may have
/// written — and an ordinary edit from the frontend must keep both.
///
/// The regression this pins: `BridgeMask::write` rebuilds the engine's mask
/// field by field, so `set_mask` used to replace the stored mask outright.
/// See also [`a_masks_per_point_feather_crosses_and_is_clamped`], which pins
/// the same field-by-field rebuild for the per-point widths.
/// Dragging a mask's opacity therefore deleted its animation, and dropped
/// exactly the unknown fields docs/10 §1.1 makes it mandatory to round-trip.
#[test]
fn editing_a_mask_keeps_what_the_bridge_cannot_describe() {
    use crate::api::layer::{BridgeMask, BridgeMaskMode, BridgeVertex};

    let (project, layer) = project_with_layer();
    let vertices = vec![
        BridgeVertex {
            x: 0.0,
            y: 0.0,
            tan_in_x: 0.0,
            tan_in_y: 0.0,
            tan_out_x: 0.0,
            tan_out_y: 0.0,
        },
        BridgeVertex {
            x: 10.0,
            y: 0.0,
            tan_in_x: 0.0,
            tan_in_y: 0.0,
            tan_out_x: 0.0,
            tan_out_y: 0.0,
        },
        BridgeVertex {
            x: 10.0,
            y: 10.0,
            tan_in_x: 0.0,
            tan_in_y: 0.0,
            tan_out_x: 0.0,
            tan_out_y: 0.0,
        },
    ];
    let mask = BridgeMask {
        id: uuid::Uuid::now_v7(),
        name: "Rectangle".into(),
        vertices: vertices.clone(),
        closed: true,
        inverted: false,
        opacity: BridgeScalar::Static(100.0),
        mode: BridgeMaskMode::Add,
        feather: BridgeScalar::Static(0.0),
        vertex_feather: Vec::new(),
        expansion: BridgeScalar::Static(0.0),
        path_keys: Vec::new(),
    };
    layer.add_mask(mask.clone()).expect("added");

    // Give the stored mask both of the things the bridge cannot carry, as a
    // newer version of Lumit (or the keyframe UI, once it exists) would.
    let key_time = lumit_core::time::Rational::new(1, 1).expect("1 s");
    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        let mut doc = lumit_core::Document::clone(&state.store.snapshot());
        let stored = doc
            .comp_mut(layer.comp_id)
            .expect("the comp")
            .layers
            .iter_mut()
            .flat_map(|l| l.masks.iter_mut())
            .find(|m| m.id == mask.id)
            .expect("the mask we just added");
        stored.path_keys = vec![lumit_core::mask::PathKeyframe {
            time: key_time,
            path: stored.path.clone(),
            interp_in: lumit_core::anim::SideInterp::Linear,
            interp_out: lumit_core::anim::SideInterp::Linear,
        }];
        stored
            .extra
            .insert("fromTheFuture".into(), serde_json::json!(7));
        state.store.replace_document(doc);
    }

    // An ordinary edit: the same thing dragging the opacity slider does. No
    // time, because an opacity drag is not a shape edit.
    layer
        .set_mask(
            BridgeMask {
                opacity: BridgeScalar::Static(40.0),
                ..mask.clone()
            },
            None,
        )
        .expect("edited");

    let state = project.state().expect("state");
    let state = state.read().expect("read");
    let doc = state.store.snapshot();
    let stored = doc
        .comp(layer.comp_id)
        .expect("the comp")
        .layers
        .iter()
        .flat_map(|l| l.masks.iter())
        .find(|m| m.id == mask.id)
        .expect("the mask survives its own edit");

    assert!(
        (stored.opacity.value_at(0.0) - 40.0).abs() < 1e-9,
        "the edit landed"
    );
    assert_eq!(
        stored.path_keys.len(),
        1,
        "an opacity edit must not delete the mask's animation"
    );
    assert_eq!(stored.path_keys[0].time, key_time);
    assert_eq!(
        stored.extra.get("fromTheFuture"),
        Some(&serde_json::json!(7)),
        "a field a newer Lumit wrote must survive an edit from this one"
    );
    drop(state);

    // **A shape edit on a keyed mask lands on the key**. Once a path is
    // animated `path` is not what the mask draws, so writing the dragged
    // vertices there would move nothing at all and the shape would look frozen
    // under the pointer.
    let dragged: Vec<BridgeVertex> = mask
        .vertices
        .iter()
        .map(|v| BridgeVertex {
            x: v.x + 25.0,
            ..*v
        })
        .collect();
    layer
        .set_mask(
            BridgeMask {
                vertices: dragged,
                ..mask.clone()
            },
            Some(BridgeRational {
                num: key_time.num(),
                den: key_time.den(),
            }),
        )
        .expect("shape edited");

    let state = project.state().expect("state");
    let state = state.read().expect("read");
    let doc = state.store.snapshot();
    let stored = doc
        .comp(layer.comp_id)
        .expect("the comp")
        .layers
        .iter()
        .flat_map(|l| l.masks.iter())
        .find(|m| m.id == mask.id)
        .expect("the mask is still there");
    assert_eq!(stored.path_keys.len(), 1, "the drag reused the key there");
    assert!(
        (stored.path_keys[0].path.vertices[0].pos.0 - (mask.vertices[0].x + 25.0)).abs() < 1e-9,
        "the dragged shape went into the key, not the ignored static path"
    );

    // The document's lock goes back before anything writes through it again:
    // `clear_mask_path_keys` below takes the write side, and a read guard still
    // alive here would sit on it for ever (docs/14: no lock held across a call
    // that takes the other side).
    drop(state);

    // **And the wireframe can find that shape**. The mask still carries
    // its old static path — `path` is not what an animated mask draws — so
    // without this the Viewer drew the shape snapping back to where it began
    // the moment the drag ended, even though the render animated correctly.
    let comp = crate::api::composition::CompositionReference::new(layer.project_id, layer.comp_id);
    let shown = comp
        .animated_mask_paths_at(0)
        .expect("the comp answers for frame 0");
    let row = shown
        .iter()
        .find(|r| r.mask == mask.id)
        .expect("an animated mask is listed");
    assert_eq!(row.layer, layer.layer_id);
    assert!(
        (row.vertices[0].x - (mask.vertices[0].x + 25.0)).abs() < 1e-9,
        "the shape shown is the keyed one, not the stale static path"
    );

    // A still mask is not listed at all: its own vertices already say where it
    // is, and sending every mask every frame is what this avoids.
    layer
        .clear_mask_path_keys(mask.id, BridgeRational { num: 0, den: 1 })
        .expect("stopped animating");
    assert!(
        comp.animated_mask_paths_at(0)
            .expect("still answers")
            .is_empty(),
        "a mask that is not animated must not be listed"
    );
}

/// A closed project is forgotten, and the channel its worker waits on is
/// dropped with it — which is how the worker learns to stop. Without this,
/// every project a process ever makes keeps a live render worker and its GPU
/// device until the process dies; the frb test suite piles up one per test,
/// and the Linux CI runner ran out of memory under them.
#[test]
fn a_closed_project_is_forgotten_and_its_worker_channel_disconnects() {
    // `close` empties the process-wide solve store, so this waits for the
    // planar tests rather than emptying one mid-read.
    let _solves = track_store_test();
    let project = LumitBridgeState::new_project(None).expect("project");

    // Stand in for `run_worker`: park the sender in the state, exactly where
    // the real worker's request channel lives, and keep the receiving end —
    // the worker's seat. Only `close` may drop that sender.
    let (sender, receiver) = std::sync::mpsc::channel::<crate::api::worker_thread::WorkerRequest>();
    {
        let state = project.state().expect("state");
        let mut state = state.write().expect("write");
        state.sender = Some(sender);
    }

    project.close().expect("closed");

    // Forgotten: every later call through the reference is a calm error.
    assert!(project.state().is_err(), "a closed project must be gone");
    assert!(
        matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ),
        "the worker's channel must disconnect when the project closes"
    );

    // Closing again is not an error: it is closed, which is what was asked.
    project.close().expect("closing a closed project is fine");
}

/// Turning flow off parks its tuning instead of dropping it, and puts the
/// policy and the tuning back in one undo step — the point of them riding the
/// same op (docs/04-RETIMING.md §10).
#[test]
fn turning_flow_off_parks_its_tuning_and_one_undo_restores_both() {
    use crate::api::retime::BridgeFlowParams;

    let (project, layer) = project_with_layer();
    layer.set_flow_enabled(true).expect("flow on");
    let tuned = BridgeFlowParams {
        engine: 0,
        resolution: 1,
        detail: 3,
        smoothness: 80.0,
        occlusion: 1,
        fallback: 1,
        hud_guard: false,
        always: false,
    };
    layer.set_flow_params(tuned.clone()).expect("tuned");

    layer.set_flow_enabled(false).expect("flow off");
    assert!(!layer.get_flow_enabled().expect("read"), "flow is off");
    assert_eq!(
        layer.get_flow_params().expect("read"),
        tuned,
        "the tuning is parked, not gone"
    );

    project.undo().expect("undone");
    assert!(layer.get_flow_enabled().expect("read"), "flow is back on");
    assert_eq!(
        layer.get_flow_params().expect("read"),
        tuned,
        "one undo step brings the policy and its tuning back together"
    );

    layer.set_flow_enabled(true).expect("already on is a no-op");
    assert_eq!(layer.get_flow_params().expect("read"), tuned);
}

/// The engine that paints the in-between frame writes and reads with the rest
/// of the Flow group, in one op (docs/impl/addons.md §6.3).
///
/// A group of settings edited one at a time would be eight round trips and
/// eight undo steps, so the engine rides the same write the others do. An
/// unknown code keeps what was there: the panel and the engine disagreeing
/// about how many engines exist is a bug, not a reason to change the user's
/// picture.
#[test]
fn the_flow_engine_writes_whole_with_the_group() {
    use crate::api::retime::BridgeFlowParams;

    let (project, layer) = project_with_layer();
    layer.set_flow_enabled(true).expect("flow on");
    assert_eq!(
        layer.get_flow_params().expect("read").engine,
        0,
        "a layer that has never been told reads as the built-in engine"
    );

    let asked = BridgeFlowParams {
        engine: 1,
        smoothness: 40.0,
        ..layer.get_flow_params().expect("read")
    };
    layer.set_flow_params(asked.clone()).expect("written");
    assert_eq!(
        layer.get_flow_params().expect("read"),
        asked,
        "the whole group came back, engine included"
    );

    project.undo().expect("undone");
    assert_eq!(
        layer.get_flow_params().expect("read").engine,
        0,
        "and one undo put the engine back with everything else"
    );

    layer.set_flow_params(asked.clone()).expect("written again");
    let nonsense = BridgeFlowParams {
        engine: 99,
        ..asked
    };
    layer.set_flow_params(nonsense).expect("written");
    assert_eq!(
        layer.get_flow_params().expect("read").engine,
        1,
        "an engine this build does not know keeps the one that was chosen"
    );
}

/// An export whose document names a model this machine cannot run refuses to
/// start, and the same export starts once the layer asks for the built-in
/// engine (docs/impl/addons.md §6.3, docs/08 §3.1).
///
/// Preview stands the built-in engine in and says so on the row. A file is
/// different: it is kept, and nothing about it afterwards says which engine
/// drew it, so the refusal is at the start and before a byte is written.
#[test]
fn an_export_refuses_a_model_engine_with_no_pack_installed() {
    use crate::api::export::{export_cancel, BridgeExportSpec};
    use crate::api::retime::BridgeFlowParams;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    layer.set_flow_enabled(true).expect("flow on");
    layer
        .set_flow_params(BridgeFlowParams {
            engine: 1,
            ..layer.get_flow_params().expect("read")
        })
        .expect("the layer asks for the model");

    let target = std::env::temp_dir().join("lumit-addon-export-probe.mp4");
    let path = target.to_string_lossy().into_owned();
    let _serial = addons_serially();
    let empty = tempfile::tempdir().expect("a folder");
    lumit_ml::store::with_dir(Some(empty.path().to_path_buf()));

    let refused = comp.start_export(BridgeExportSpec::default(), path.clone());
    assert!(
        matches!(refused, Err(BridgeError::AddonMissing)),
        "an export that names a missing addon is refused: {refused:?}"
    );
    assert!(!target.exists(), "and nothing was written");
    assert!(
        comp.addon_needed(),
        "and the dialogue can tell this refusal from the encoder's"
    );

    // Both footer actions of the export dialogue are `queue_export`, so the
    // pre-flight has to stand there too: an item added now is an item that
    // renders this same snapshot later.
    for start in [true, false] {
        let queued = comp.queue_export(BridgeExportSpec::default(), path.clone(), start);
        assert!(
            matches!(queued, Err(BridgeError::AddonMissing)),
            "the queue refuses it as well: {queued:?}"
        );
    }

    // The same document with the built-in engine gets as far as the exporter,
    // which on a machine with no graphics adapter says so. Either way it is
    // past the pre-flight, which is what this proves.
    layer
        .set_flow_params(BridgeFlowParams {
            engine: 0,
            ..layer.get_flow_params().expect("read")
        })
        .expect("back to the built-in engine");
    assert!(
        !comp.addon_needed(),
        "a document that asks for no model needs no addon, on any machine"
    );
    let started = comp.start_export(BridgeExportSpec::default(), path);
    lumit_ml::store::with_dir(None);
    assert!(
        started.is_ok() || matches!(started, Err(BridgeError::ExportFailed(_))),
        "an export either starts or explains itself: {started:?}"
    );
    export_cancel();
    std::fs::remove_file(&target).ok();
}

// --- Camera track: the effect's surface across the seam -------------------

/// A project, a comp, a footage layer carrying an enabled Camera track, and a
/// written-down solve published for that footage's media.
///
/// The solve is **written down, not computed**: `lumit-render`'s own tests drive
/// the analysis over a rendered shot, and repeating that here would be measuring
/// the tracker again rather than the seam. The camera sits at the world origin
/// looking down +z with a focal of 100, so every projection below is arithmetic
/// anyone reading the test can do in their head.
fn a_tracked_layer() -> (
    crate::api::project::ProjectReference,
    CompositionReference,
    LayerReference,
    Uuid,
) {
    use lumit_track::{CameraSolve, PoseSource, ScenePoint, SolveSegment, SolvedPose};

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project
        .new_composition(
            "Scene".into(),
            Some(BridgeCompSettings {
                name: "Scene".into(),
                width: 1920,
                height: 1080,
                fps_num: 25,
                fps_den: 1,
                // Two seconds: the bake writes one key per frame, and a
                // half-minute comp would be fifty keyframes' worth of test for
                // no extra claim.
                duration: BridgeRational { num: 2, den: 1 },
                background: [0.0, 0.0, 0.0, 1.0],
                shutter_angle: 180.0,
                motion_blur_samples: 16,
            }),
        )
        .expect("comp");
    let footage = project
        .import_footage("C:/clips/tracked.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false, None)
        .expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    layer
        .add_effect(lumit_core::track::CAMERA_TRACK.to_owned())
        .expect("the Camera track is a builtin");

    // Frame n moves the camera n units along x, so a walk that lands on the
    // wrong frame cannot pass by accident.
    let poses: Vec<SolvedPose> = (0..50)
        .map(|frame| SolvedPose {
            frame,
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            position: [frame as f64, 0.0, 0.0],
            segment: 0,
            focal_px: 100.0,
            mean_reprojection_px: 0.1,
            source: PoseSource::Keyframe,
        })
        .collect();
    let solve = CameraSolve {
        poses,
        segments: vec![SolveSegment {
            first_frame: 0,
            last_frame: 49,
            focal_px: 100.0,
            ramp: false,
        }],
        // One near, one far, so the depth cue has a spread to normalise over.
        points: vec![
            ScenePoint {
                track: 7,
                position: [10.0, 20.0, 100.0],
            },
            ScenePoint {
                track: 9,
                position: [30.0, 40.0, 200.0],
            },
        ],
        keyframes: vec![0, 49],
        mean_reprojection_px: 0.25,
        notes: Vec::new(),
    };
    let media = footage.id();
    // Fifty frames solved out of a fifty-frame clip: a whole track, so the
    // partial reading below has something honest to be measured against.
    lumit_render::track::publish(media, 25.0, 50, solve);
    (project, comp, layer, media)
}

/// A Null made from picked points lands at their mean solved position, in 3D —
/// and one undo step puts it back.
#[test]
fn a_null_lands_at_the_mean_of_the_points_it_was_made_from() {
    use crate::api::layer::BridgeLayerKind;
    use crate::api::track::{add_layer_at_points, add_solved_camera};

    let (project, comp, layer, _media) = a_tracked_layer();
    // A camera to face: without one the layer is made square to the comp,
    // which is the honest fallback but not the claim being made here.
    add_solved_camera(layer).expect("a linked camera");

    let made = add_layer_at_points(layer, vec![7, 9], 0, false).expect("a null");
    let transform = made.get_transform().expect("transform");
    let still = |s: &BridgeScalar| match s {
        BridgeScalar::Static(v) => *v,
        _ => panic!("a fresh layer's transform is static"),
    };
    assert_eq!(made.get_kind().expect("kind"), BridgeLayerKind::NullLayer);
    assert!((still(&transform.position_x) - 20.0).abs() < 1e-9);
    assert!((still(&transform.position_y) - 30.0).abs() < 1e-9);
    assert!((still(&transform.position_z) - 150.0).abs() < 1e-9);
    assert!(
        made.get_switches().expect("switches").three_d,
        "a position in z means nothing on a layer that is not 3D"
    );

    // Naming nothing that was solved is a refusal, not a layer at the origin.
    assert!(matches!(
        add_layer_at_points(layer, vec![404], 0, false),
        Err(BridgeError::NoSolve)
    ));

    let before = comp.get_layers().expect("layers").len();
    project.undo().expect("undo");
    assert_eq!(
        comp.get_layers().expect("layers").len(),
        before - 1,
        "the layer went in as one step and comes out as one"
    );
}

/// The badge reads the link, and Convert to keyframes bakes the motion and
/// ends it — after which the camera is an ordinary one the user edits.
#[test]
fn a_linked_camera_reads_derived_and_converts_to_keyframes() {
    use crate::api::track::{
        add_solved_camera, camera_link, convert_camera_to_keyframes, BridgeLinkState,
    };

    let (_project, _comp, layer, _media) = a_tracked_layer();
    let camera = add_solved_camera(layer).expect("a linked camera");

    let link = camera_link(camera, 0);
    assert_eq!(link.state, BridgeLinkState::Derived);
    assert_eq!(link.tracked, Some(layer.layer_id));

    // Inside the solved range the pose is derived; the comp is fifty frames
    // long and the solve is fifty frames, so nothing is held here.
    assert_eq!(camera_link(camera, 49).state, BridgeLinkState::Derived);

    convert_camera_to_keyframes(camera).expect("baked");
    let after = camera_link(camera, 0);
    assert_eq!(after.state, BridgeLinkState::Unlinked);
    assert_eq!(after.tracked, None);
    let transform = camera.get_transform().expect("transform");
    let BridgeScalar::Keyframed(keys) = &transform.position_x else {
        panic!("the bake writes a key per frame");
    };
    assert_eq!(keys.len(), 50, "two seconds at twenty-five frames");
    // The baked motion is the derived motion: frame five was five units along.
    assert!((keys[5].value - 5.0).abs() < 1e-9, "{}", keys[5].value);
}

/// **Track once, then nudge** across the seam: a linked camera takes a
/// transform edit, the edit reads back as a correction on top of the solve, the
/// dot lights on both rows that draw it, and Clear corrections puts it back.
#[test]
fn a_correction_shows_on_both_rows_and_clears() {
    use crate::api::layer::BridgeTransformProp;
    use crate::api::track::{
        add_solved_camera, camera_link, clear_camera_corrections, BridgeLinkState,
    };

    let (_project, comp, layer, _media) = a_tracked_layer();
    let camera = add_solved_camera(layer).expect("a linked camera");

    // The rows the dot is drawn on: the camera's own, and the tracked layer's
    // Camera track card. Neither says anything yet.
    let corrected = |of: LayerReference| of.get_info().expect("info").track_corrected;
    assert!(!corrected(camera), "nobody has nudged it");
    assert!(!corrected(layer), "and so the effect's row says nothing");

    // Frame five is five units along; the nudge is thirty more.
    let before = camera
        .get_transform()
        .expect("transform")
        .position_x
        .clone();
    let BridgeScalar::Static(base) = before else {
        panic!("a fresh camera's position is a plain number");
    };
    camera
        .set_transform(
            BridgeTransformProp::PositionX,
            BridgeScalar::Static(base + 30.0),
        )
        .expect("a linked camera takes a transform edit");

    assert!(corrected(camera), "the camera's own row");
    assert!(corrected(layer), "and the Camera track's row beside it");
    assert_eq!(
        camera_link(camera, 5).state,
        BridgeLinkState::Derived,
        "correcting a camera does not stop it following the shot"
    );

    // What the pose actually came to: the solve, plus the nudge. Read through
    // the bake, which is the one crossing that hands the derived numbers back.
    crate::api::track::convert_camera_to_keyframes(camera).expect("baked");
    let BridgeScalar::Keyframed(keys) = &camera.get_transform().expect("transform").position_x
    else {
        panic!("the bake writes a key per frame");
    };
    assert!(
        (keys[5].value - 35.0).abs() < 1e-9,
        "five from the solve and thirty from the nudge: {}",
        keys[5].value
    );

    // A second camera, to clear rather than bake.
    let camera = add_solved_camera(layer).expect("another linked camera");
    assert!(
        matches!(
            clear_camera_corrections(camera),
            Err(BridgeError::NotLinked)
        ),
        "an untouched camera has nothing to clear"
    );
    camera
        .set_transform(BridgeTransformProp::RotationY, BridgeScalar::Static(4.0))
        .expect("nudged");
    assert!(corrected(camera));
    clear_camera_corrections(camera).expect("cleared");
    assert!(!corrected(camera), "the dot goes out with the correction");
    assert_eq!(
        camera_link(camera, 5).tracked,
        Some(layer.layer_id),
        "clearing the nudge must not clear the track"
    );

    // A layer with no camera on it, and no track, says nothing either way.
    let solid = comp.add_solid_layer(None).expect("a solid");
    assert!(!corrected(solid));
}

/// A segmentation prompt crosses the seam, lands in the document as a whole
/// edit, and the status row counts it (docs/impl/addons.md §6.2, §11 test 10).
///
/// The taps ride the ordinary effect-stack commit, as the strokes do, so a
/// prompt is one `SetLayerEffects` and one undo step; the first one sets the
/// base frame, which is what makes Propagate answerable on a brush nobody
/// scribbled on.
#[test]
fn a_prompt_lands_in_the_document_and_the_status_counts_it() {
    use crate::api::roto::roto_status;

    let (_project, comp, layer, _media) = a_tracked_layer();
    layer
        .add_effect(lumit_core::roto::ROTO_BRUSH.to_owned())
        .expect("the Roto brush is a builtin");
    let stack = layer.get_effects().expect("stack");
    let effect = stack.last().expect("the Roto brush").id();

    let before = roto_status(layer, effect).expect("a status");
    assert_eq!((before.strokes, before.prompts), (0, 0));
    assert_eq!(before.base_frame, None, "nothing has been asked of it yet");
    assert!(
        !before.segments,
        "the seed row reads Strokes until somebody moves it, and the card is \
         told so rather than guessing from the tap count"
    );

    let mut stack = layer.get_effects().expect("stack");
    let brush = stack.last_mut().expect("the Roto brush");
    brush
        .roto_add_prompt(vec![12.0, 34.0], vec![1], 18)
        .expect("one tap on the subject");
    brush
        .roto_add_prompt(vec![50.0, 60.0], vec![0], 18)
        .expect("and one against it");
    // A tap nobody can read never reaches the document.
    assert!(matches!(
        brush.roto_add_prompt(vec![1.0], vec![1], 18),
        Err(BridgeError::InvalidParam)
    ));
    layer.set_effects(stack, None).expect("committed");

    let after = roto_status(layer, effect).expect("a status");
    assert_eq!(after.prompts, 2, "both taps are in the document");
    assert_eq!(after.strokes, 0, "and neither of them is a stroke");
    assert_eq!(
        after.base_frame,
        Some(18),
        "the first tap sets the base frame"
    );

    // A tap away from the base frame is refused here rather than fenced in the
    // Viewer: the model is only ever asked about the base, so a prompt stored
    // anywhere else would be hashed into every frame's name on that side and
    // then never read (docs/impl/addons.md §6.2).
    let mut stack = layer.get_effects().expect("stack");
    let brush = stack.last_mut().expect("the Roto brush");
    assert!(matches!(
        brush.roto_add_prompt(vec![12.0, 34.0], vec![1], 40),
        Err(BridgeError::InvalidParam)
    ));

    // And moving the base takes the taps the new one can never read with it, in
    // this same staged edit, so one undo brings both back.
    brush.roto_set_base_frame(Some(40)).expect("the base moves");
    layer.set_effects(stack, None).expect("committed");
    let moved = roto_status(layer, effect).expect("a status");
    assert_eq!(moved.base_frame, Some(40));
    assert_eq!(
        moved.prompts, 0,
        "taps stranded on the old base would rename mattes and change no picture"
    );

    // An effect that is not a Roto brush refuses the call rather than growing a
    // block nothing will ever read.
    let mut other = layer.get_effects().expect("stack");
    assert!(matches!(
        other[0].roto_add_prompt(vec![1.0, 2.0], vec![1], 0),
        Err(BridgeError::InvalidEffect)
    ));
    let solid = comp.add_solid_layer(None).expect("a solid");
    assert!(matches!(
        roto_status(solid, effect),
        Err(BridgeError::InvalidEffect)
    ));
}

// ---------------------------------------------------------------------------
// The Project panel's five engine answers, across the seam (docs/07 §3.1,
// docs/15 §12A.3a).
// ---------------------------------------------------------------------------

/// The bottom bar's Folder button. One call, one undo step, and both of the
/// engine's decisions — the default name and the filing — honoured through it.
#[test]
fn new_folder_names_itself_files_itself_and_undoes_in_one_step() {
    let (project, folder, _filed, _loose) = project_with_folder();
    let ItemReference::Folder(parent) = &folder else {
        panic!("the fixture built a folder");
    };

    // A blank name takes the next unused "Folder N".
    let made = project.new_folder(String::new(), None).expect("a folder");
    assert_eq!(
        ItemReference::Folder(FolderReference::new(project.id, made.id))
            .name()
            .expect("name"),
        "Folder 1"
    );

    // Filed inside a parent, and the whole thing is one undo step: undoing
    // takes the folder away rather than leaving it behind unfiled.
    let filed = project
        .new_folder("Renders".into(), Some(parent.id))
        .expect("a filed folder");
    let filed_ref = ItemReference::Folder(FolderReference::new(project.id, filed.id));
    assert!(
        parent
            .get_children()
            .expect("children")
            .iter()
            .any(|c| c.equals(&filed_ref)),
        "the new folder is listed by its parent"
    );
    project.undo().expect("undone");
    assert!(
        !parent
            .get_children()
            .expect("children")
            .iter()
            .any(|c| c.equals(&filed_ref)),
        "one undo takes the folder and its filing together"
    );

    // A parent that is not a folder any more leaves it at the root rather
    // than refusing to make it at all.
    let orphan = project
        .new_folder("Loose".into(), Some(Uuid::now_v7()))
        .expect("still made");
    let orphan_ref = ItemReference::Folder(FolderReference::new(project.id, orphan.id));
    assert!(project
        .get_items()
        .expect("roots")
        .iter()
        .any(|r| r.equals(&orphan_ref)));
}

/// The flowchart's reading of a comp: what places it and what it places,
/// by name, with a mark where the nesting carries on.
#[test]
fn get_flow_names_the_comps_either_side() {
    let (project, ..) = project_with_folder();
    let comp = |name: &str| project.new_composition(name.into(), None).expect("comp");
    let (film, shot, plate, grain) = (comp("Film"), comp("Shot"), comp("Plate"), comp("Grain"));
    film.add_precomp_layer(&shot, None).expect("nested");
    shot.add_precomp_layer(&plate, None).expect("nested");
    plate.add_precomp_layer(&grain, None).expect("nested");

    let flow = shot.get_flow().expect("flow");
    assert_eq!(flow.name, "Shot");
    let read = |links: &[crate::api::composition::BridgeCompFlowLink]| {
        links
            .iter()
            .map(|l| (l.comp.clone(), l.name.clone(), l.more))
            .collect::<Vec<_>>()
    };
    assert_eq!(read(&flow.used_by), [(film.clone(), "Film".into(), false)]);
    assert_eq!(read(&flow.uses), [(plate, "Plate".into(), true)]);

    let top = film.get_flow().expect("flow");
    assert!(top.used_by.is_empty(), "nothing places the film");
    assert_eq!(read(&top.uses), [(shot, "Shot".into(), true)]);
}

// ---------------------------------------------------------------------------
// The Timeline's three (docs/15 §6.3, §12A.1).
// ---------------------------------------------------------------------------

/// A marker's span crosses as frames and comes back unharmed, and a marker
/// nobody resized keeps the exact duration the document holds even when the
/// panel writes the whole list for some other reason.
#[test]
fn a_markers_span_crosses_as_frames_and_survives_a_rename() {
    use crate::api::composition::BridgeMarker;

    let project = LumitBridgeState::new_project(None).expect("a project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");

    comp.set_markers(vec![
        BridgeMarker {
            id: Uuid::now_v7(),
            time: BridgeRational { num: 0, den: 1 },
            label: "moment".into(),
            duration_frames: None,
            colour: None,
            is_beat: false,
        },
        BridgeMarker {
            id: Uuid::now_v7(),
            time: BridgeRational { num: 1, den: 1 },
            label: "span".into(),
            duration_frames: Some(12),
            colour: Some(3),
            is_beat: false,
        },
    ])
    .expect("written");

    let read = comp.get_markers().expect("markers");
    assert_eq!(read[0].duration_frames, None, "a moment stays a moment");
    assert_eq!(read[1].duration_frames, Some(12), "and a span its length");

    // Renaming writes the whole list back. The span must not be quantised or
    // dropped by a write that never touched it.
    let renamed: Vec<BridgeMarker> = read
        .into_iter()
        .map(|m| BridgeMarker {
            label: format!("{} renamed", m.label),
            ..m
        })
        .collect();
    comp.set_markers(renamed).expect("written");
    let read = comp.get_markers().expect("markers");
    assert_eq!(read[1].duration_frames, Some(12));
    assert_eq!(read[1].label, "span renamed");
    assert_eq!(read[1].colour, Some(3), "and the colour it was given");
    assert_eq!(read[0].colour, None, "a marker nobody coloured stays plain");

    // Nought frames is a moment, which is what "no span" means everywhere.
    let mut back = comp.get_markers().expect("markers");
    back[1].duration_frames = Some(0);
    comp.set_markers(back).expect("written");
    assert_eq!(
        comp.get_markers().expect("markers")[1].duration_frames,
        None
    );
}

// ---------------------------------------------------------------------------
// The Effect controls' unit rider and vector-pair chain (docs/15 §12A.3).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The layer driver graph (docs/impl/node-graph.md §5).
// ---------------------------------------------------------------------------

use crate::api::graph::{
    BridgeGraphEdge, BridgeGraphWiring, BridgeInputRef, BridgeLayerGraph, BridgeNodePosition,
    BridgeNodeRef, BridgeOutputRef, BridgePortType,
};

/// A comp with one solid, ready to be wired.
fn layer_to_wire() -> (ProjectReference, LayerReference) {
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer(None).expect("a solid");
    (project, layer)
}

/// The ids of the boxes the graph draws for the image chain, in draw order.
fn image_chain(graph: &BridgeLayerGraph) -> Vec<Uuid> {
    graph
        .nodes
        .iter()
        .filter_map(|n| match n.node {
            BridgeNodeRef::Effect(id) => Some(id),
            _ => None,
        })
        .collect()
}

/// One driver wired into one effect parameter — the shape most of these bend.
fn wiggle_into_blur(layer: &LayerReference) -> (Uuid, Uuid) {
    layer.add_effect("blur".into()).expect("a blur");
    let blur = layer.get_effects().expect("stack")[0].id();
    let wiggle = layer.new_driver("wiggle".into()).expect("a wiggle");
    let wiggle_id = wiggle.id();
    layer
        .set_graph(
            vec![wiggle],
            BridgeGraphWiring {
                out_unwired: false,
                edges: vec![BridgeGraphEdge {
                    from: BridgeOutputRef::Driver {
                        node: wiggle_id,
                        port: "value".into(),
                    },
                    to: BridgeInputRef::Param {
                        node: BridgeNodeRef::Effect(blur),
                        port: "radius".into(),
                    },
                }],
                layout: vec![BridgeNodePosition {
                    node: BridgeNodeRef::Driver(wiggle_id),
                    x: -120.0,
                    y: 40.0,
                }],
                exposed: vec![BridgeNodeRef::Effect(blur)],
                groups: Vec::new(),
            },
        )
        .expect("a number into a number");
    (blur, wiggle_id)
}

/// **The stack view never lies** (§9.2). The read model derives the image chain
/// from `Layer::effects` and stores none of it, so whatever the stack is — in
/// whatever order, however edited — the graph reports exactly that, box for box.
///
/// Walked over a set of arrangements rather than one example: the invariant is
/// about every stack, and an example only pins the one it happens to hold.
#[test]
fn the_graph_reports_the_effect_stack_as_the_image_chain() {
    let (_project, layer) = layer_to_wire();

    for name in ["blur", "levels", "glow", "fill"] {
        layer.add_effect(name.to_owned()).expect("added");
    }
    let ids = |layer: &LayerReference| -> Vec<Uuid> {
        layer
            .get_effects()
            .expect("stack")
            .iter()
            .map(BridgeEffectInstance::id)
            .collect()
    };
    assert_eq!(image_chain(&layer.get_graph().expect("graph")), ids(&layer));

    // Every reordering of it still reads back as the list it is.
    for to in 0..4 {
        let staged = layer.get_effects().expect("stack");
        layer.reorder_effect(&staged[0], to).expect("reordered");
        assert_eq!(
            image_chain(&layer.get_graph().expect("graph")),
            ids(&layer),
            "the chain is the list, whatever order the list is in"
        );
    }

    // And so does a removal.
    let staged = layer.get_effects().expect("stack");
    layer.remove_effect(&staged[1]).expect("removed");
    assert_eq!(image_chain(&layer.get_graph().expect("graph")), ids(&layer));

    let graph = layer.get_graph().expect("graph");
    assert_eq!(
        graph.nodes.first().map(|n| n.node),
        Some(BridgeNodeRef::Source),
        "the Source is the first box drawn"
    );
    assert_eq!(
        graph.nodes.last().map(|n| n.node),
        Some(BridgeNodeRef::Out),
        "and the Layer out closes the chain — there being no driver here"
    );
}

/// Adding a driver, wiring it, placing it and growing a box are **one** write
/// and therefore one undo step (§3).
#[test]
fn a_driver_is_added_wired_and_undone_in_one_step() {
    let (project, layer) = layer_to_wire();
    let (blur, wiggle) = wiggle_into_blur(&layer);

    let graph = layer.get_graph().expect("graph");
    let node = graph
        .nodes
        .iter()
        .find(|n| n.node == BridgeNodeRef::Driver(wiggle))
        .expect("the driver is drawn");
    assert_eq!(node.match_name, "wiggle");
    assert_eq!(node.label, "Wiggle", "English on the wire");
    assert!(node.enabled);
    assert_eq!(
        node.outputs
            .iter()
            .map(|p| (p.id.as_str(), p.label.as_str(), p.port_type, p.wired))
            .collect::<Vec<_>>(),
        vec![("value", "Value", BridgePortType::Number, true)],
        "its one output is wired, and names itself in English"
    );
    assert_eq!(
        node.inputs
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["amount", "frequency"],
        "a socket per parameter a wire could feed"
    );

    // The socket the wire lands on says so; its siblings do not.
    let blur_node = graph
        .nodes
        .iter()
        .find(|n| n.node == BridgeNodeRef::Effect(blur))
        .expect("the blur is drawn");
    assert!(
        blur_node.inputs.iter().any(|p| p.id == "radius" && p.wired),
        "the driven parameter's socket is filled"
    );
    assert_eq!(
        blur_node.inputs.iter().filter(|p| p.wired).count(),
        2,
        "the radius and the picture coming in — and nothing else"
    );

    // The stored half comes back exactly as it was written.
    assert_eq!(graph.wiring.exposed, vec![BridgeNodeRef::Effect(blur)]);
    assert_eq!(graph.wiring.layout.len(), 1);
    assert_eq!(graph.wiring.layout[0].x, -120.0);

    // One gesture, one undo step: the node and its wire go together.
    project.undo().expect("undone");
    let graph = layer.get_graph().expect("graph");
    assert!(graph.wiring.edges.is_empty(), "the undo took the wire");
    assert!(
        !graph
            .nodes
            .iter()
            .any(|n| matches!(n.node, BridgeNodeRef::Driver(_))),
        "and the node with it"
    );
}

/// **Every refusal arrives as its own calm sentence** (§1.5). A graph that
/// breaks a rule is declined whole; the document is left exactly as it was.
#[test]
fn a_broken_graph_is_refused_with_the_engines_own_words() {
    let (_project, layer) = layer_to_wire();
    let (blur, _) = wiggle_into_blur(&layer);
    let before = layer.get_graph().expect("graph");

    let drivers = |layer: &LayerReference| layer.get_graph_drivers().expect("drivers");
    let param = |node, port: &str| BridgeInputRef::Param {
        node,
        port: port.to_owned(),
    };
    let from = |id: Uuid, port: &str| BridgeOutputRef::Driver {
        node: id,
        port: port.to_owned(),
    };
    let wiring = |edges: Vec<BridgeGraphEdge>| BridgeGraphWiring {
        edges,
        layout: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
        out_unwired: false,
    };

    let id = drivers(&layer)[0].id();
    let cases: Vec<(Vec<BridgeEffectInstance>, BridgeGraphWiring, &str)> = vec![
        (
            Vec::new(),
            wiring(vec![BridgeGraphEdge {
                from: from(id, "value"),
                to: param(BridgeNodeRef::Effect(blur), "radius"),
            }]),
            "a wire names a node this layer does not have",
        ),
        (
            drivers(&layer),
            wiring(vec![BridgeGraphEdge {
                from: from(id, "value"),
                to: param(BridgeNodeRef::Effect(blur), "no_such_parameter"),
            }]),
            "a wire names a port that does not exist",
        ),
        (
            drivers(&layer),
            wiring(vec![BridgeGraphEdge {
                from: from(id, "value"),
                to: BridgeInputRef::Matte { effect: blur },
            }]),
            "a wire joins two ports of different types",
        ),
        (
            drivers(&layer),
            wiring(vec![
                BridgeGraphEdge {
                    from: from(id, "value"),
                    to: param(BridgeNodeRef::Effect(blur), "radius"),
                },
                BridgeGraphEdge {
                    from: from(id, "value"),
                    to: param(BridgeNodeRef::Effect(blur), "radius"),
                },
            ]),
            "a socket cannot take a second wire",
        ),
        (
            drivers(&layer),
            wiring(vec![BridgeGraphEdge {
                from: from(id, "value"),
                to: param(BridgeNodeRef::Driver(id), "amount"),
            }]),
            "the wire would close a loop",
        ),
    ];

    for (nodes, wiring, sentence) in cases {
        let refusal = layer
            .set_graph(nodes, wiring)
            .expect_err("the rule is enforced");
        assert_eq!(
            refusal.to_string(),
            sentence,
            "the refusal is the engine's own calm sentence, fit for the status line"
        );
        assert_eq!(
            layer.get_graph().expect("graph").wiring,
            before.wiring,
            "a refused write leaves the document exactly as it was"
        );
    }
}

// ---------------------------------------------------------------------------
// The points edge (docs/impl/points-stream.md §1, §4.2).
// ---------------------------------------------------------------------------

/// A proxy is attached, switched and detached over the seam, and every row
/// state the Project panel draws is readable from one call.
#[test]
fn a_proxy_is_attached_switched_and_detached_over_the_seam() {
    let (project, _folder, filed, _loose) = project_with_folder();
    let ItemReference::Footage(footage) = &filed else {
        panic!("the fixture filed a footage item");
    };

    assert_eq!(
        footage.get_proxy().expect("read"),
        None,
        "an item with no proxy has no row to draw"
    );
    assert!(
        footage.set_use_proxy(true).is_err(),
        "there is no tick to set on a row with no proxy"
    );
    assert!(
        matches!(
            footage.set_proxy("   ".into()),
            Err(BridgeError::MediaPathUnresolved)
        ),
        "a blank path is refused rather than attached"
    );

    footage
        .set_proxy("clips/filed_proxy.mov".into())
        .expect("attached");
    let attached = footage.get_proxy().expect("read").expect("a proxy");
    assert!(attached.path.ends_with("filed_proxy.mov"));
    assert!(attached.enabled, "attaching switches it on");
    assert!(attached.in_use, "and the project switch starts on");

    // The project's master switch turns the reading off without disturbing the
    // item's own tick — the "show me what I am delivering" switch.
    project.set_use_proxies(false).expect("master off");
    assert!(!project.use_proxies().expect("read"));
    let off = footage.get_proxy().expect("read").expect("still attached");
    assert!(off.enabled, "the item's own tick is untouched");
    assert!(!off.in_use, "but nothing reads it");

    project.set_use_proxies(true).expect("master on");
    footage.set_use_proxy(false).expect("this one item off");
    let one_off = footage.get_proxy().expect("read").expect("still attached");
    assert!(!one_off.enabled);
    assert!(!one_off.in_use);

    footage.clear_proxy().expect("detached");
    assert_eq!(footage.get_proxy().expect("read"), None);

    // Each of those was one op, so one undo puts the proxy back.
    project.undo().expect("undone");
    assert!(footage.get_proxy().expect("read").is_some());
}

// ---------------------------------------------------------------------------
// Colour management (docs/impl/ocio.md §6.1). The seam's half:
// the summary a picker is built from, the two edits, deliverability, and the
// export's pre-queue refusal.
// ---------------------------------------------------------------------------

/// A small, complete config: one space that is not the reference, one display
/// with one view, and the roles the resolution walk reads.
const GOOD_CONFIG: &str = r#"
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

/// A project holding one footage item, and a directory to write configs into.
fn project_with_footage() -> (ProjectReference, FootageReference, tempfile::TempDir) {
    let project = crate::api::state::LumitBridgeState::new_project(None).expect("a new project");
    let id = Uuid::now_v7();
    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        state
            .store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(FootageItem {
                    sequence: None,
                    id,
                    name: "shot.mov".into(),
                    media: MediaRef {
                        relative_path: "shot.mov".into(),
                        absolute_path: String::new(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    extra: serde_json::Map::new(),
                    colour_space: None,
                    source_layer: None,
                })),
            })
            .expect("seeded");
    }
    let footage = FootageReference::new(project.id, id);
    (project, footage, tempfile::tempdir().expect("a directory"))
}

fn write_config(dir: &tempfile::TempDir, text: &str) -> String {
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, text).expect("a config on disk");
    path.to_string_lossy().into_owned()
}

/// A config that is not there does not stop anything: the project keeps every
/// name it was given, and the refusal crosses as an id plus its facts so the
/// frontend can write the sentence in the reader's own language.
#[test]
fn a_missing_config_refuses_by_id_and_keeps_every_name() {
    let (project, footage, dir) = project_with_footage();
    footage
        .set_colour_space(Some("srgb_texture".into()))
        .expect("a space named");
    let gone = dir
        .path()
        .join("not-here.ocio")
        .to_string_lossy()
        .into_owned();
    project
        .set_colour_config(Some(gone))
        .expect("a config named");

    let summary = project.colour_summary().expect("a summary");
    assert!(!summary.loaded);
    assert_eq!(summary.problem, "config_unreadable");
    assert_eq!(
        summary
            .problem_args
            .iter()
            .find(|a| a.name == "path")
            .map(|a| a.value.ends_with("not-here.ocio")),
        Some(true),
        "{:?}",
        summary.problem_args
    );
    assert!(
        !summary.problem_english.is_empty(),
        "the engine's own words ride along for a frontend with no sentence"
    );
    assert!(
        summary.spaces.is_empty(),
        "an unusable config offers nothing"
    );
    assert_eq!(
        footage.colour_space().expect("still readable"),
        Some("srgb_texture".to_string()),
        "a name is the user's statement about the file and is never dropped"
    );
}

/// The half of the preview-and-delivery asymmetry that says no: a space is
/// deliverable only when the config that defines it is loaded and the
/// transform to it bakes.
#[test]
fn only_a_loaded_config_can_deliver_its_spaces() {
    let (project, _footage, dir) = project_with_footage();
    let deliverable = |name: &str| {
        project
            .can_deliver_colour_space(name.to_string())
            .expect("asked")
    };

    assert!(
        !deliverable("out_srgb"),
        "with no config named there is nothing to deliver into"
    );

    let path = write_config(&dir, GOOD_CONFIG);
    project
        .set_colour_config(Some(path.clone()))
        .expect("a config named");
    assert!(deliverable("out_srgb"));
    assert!(!deliverable("no_such_space"));

    std::fs::remove_file(&path).expect("the config moves away");
    assert!(
        !deliverable("out_srgb"),
        "the same name a moment after the config went: a delivery refuses where a preview degrades"
    );
}

/// The sweep still refuses to touch a healthy item: a file that is where the
/// project says it is stays put, whatever the picked file's move implies.
#[test]
fn relinking_leaves_an_item_whose_file_is_still_there_alone() {
    let root = tempfile::tempdir().expect("a temp dir");
    let old = root.path().join("old");
    let new = root.path().join("new");
    std::fs::create_dir_all(&old).expect("dirs");
    std::fs::create_dir_all(&new).expect("dirs");
    std::fs::write(old.join("healthy.mov"), b"here").expect("file");
    std::fs::write(new.join("healthy.mov"), b"decoy").expect("file");
    std::fs::write(new.join("moved.mov"), b"moved").expect("file");

    let project = LumitBridgeState::new_project(None).expect("project");
    let moved = project
        .import_footage(old.join("moved.mov").to_string_lossy().into())
        .expect("imported");
    let healthy = project
        .import_footage(old.join("healthy.mov").to_string_lossy().into())
        .expect("imported");

    moved
        .relink(new.join("moved.mov").to_string_lossy().into_owned())
        .expect("relinked");

    let proj = healthy.project().expect("project");
    let p = proj.read().expect("read");
    let doc = p.store.snapshot();
    let Some(ProjectItem::Footage(f)) = doc.item(healthy.id()) else {
        panic!("expected footage");
    };
    assert_eq!(
        std::path::PathBuf::from(&f.media.absolute_path),
        old.join("healthy.mov"),
        "an item that resolves is never repointed"
    );
}

/// **A mask's per-point feather widths cross the bridge, and each is clamped
/// exactly as the one width is**.
///
/// The clamp is the half worth pinning: `BridgeMask::write` rebuilds the mask
/// field by field, so a list arriving with a negative width — or one wide
/// enough to ask for a distance field the size of a continent — would be
/// written straight into the document and render wrongly for ever after.
#[test]
fn a_masks_per_point_feather_crosses_and_is_clamped() {
    use crate::api::layer::{BridgeMask, BridgeMaskMode, BridgeVertex};

    let (_project, layer) = project_with_layer();
    let corner = |x: f64, y: f64| BridgeVertex {
        x,
        y,
        tan_in_x: 0.0,
        tan_in_y: 0.0,
        tan_out_x: 0.0,
        tan_out_y: 0.0,
    };
    let mask = BridgeMask {
        id: uuid::Uuid::now_v7(),
        name: "Rectangle".into(),
        vertices: vec![
            corner(0.0, 0.0),
            corner(10.0, 0.0),
            corner(10.0, 10.0),
            corner(0.0, 10.0),
        ],
        closed: true,
        inverted: false,
        opacity: BridgeScalar::Static(100.0),
        mode: BridgeMaskMode::Lighten,
        feather: BridgeScalar::Static(4.0),
        vertex_feather: vec![
            BridgeScalar::Static(0.0),
            BridgeScalar::Static(30.0),
            // Both ends of the clamp, in one list.
            BridgeScalar::Static(-5.0),
            BridgeScalar::Static(9_999_999.0),
        ],
        expansion: BridgeScalar::Static(0.0),
        path_keys: Vec::new(),
    };
    layer.add_mask(mask.clone()).expect("added");

    let stored = layer.get_masks().expect("read back");
    let stored = stored.first().expect("the mask");
    assert_eq!(
        stored.mode,
        BridgeMaskMode::Lighten,
        "the mode came back as itself"
    );
    let widths: Vec<f64> = stored
        .vertex_feather
        .iter()
        .map(|s| match s {
            BridgeScalar::Static(v) => *v,
            other => panic!("a still width read back as {other:?}"),
        })
        .collect();
    assert_eq!(
        widths,
        vec![0.0, 30.0, 0.0, 5000.0],
        "a width below zero and one past the ceiling are both brought inside"
    );

    // And an ordinary edit does not lose them, for the same reason an opacity
    // drag must not lose the path keys.
    layer
        .set_mask(
            BridgeMask {
                opacity: BridgeScalar::Static(40.0),
                ..stored.clone()
            },
            None,
        )
        .expect("edited");
    let after = layer.get_masks().expect("read back");
    assert_eq!(
        after.first().map(|m| m.vertex_feather.len()),
        Some(4),
        "an opacity edit dropped the per-point widths"
    );
}

/// The solve store `lumit_render::track` keeps is one per process, and
/// `ProjectReference::close` empties the whole of it: this project's solves
/// and every other's alike. Cargo runs tests in parallel threads within one
/// process, so a close in one thread wiped the track a planar test had just
/// published and the read that followed found nothing. Every test that
/// publishes a solve or closes a project takes this first.
static TRACK_STORE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Hold the process-wide solve store for the length of a test.
///
/// Reachable from the crate's other test modules, since every one of them can
/// close a project and so empty the store.
pub(crate) fn track_store_test() -> std::sync::MutexGuard<'static, ()> {
    TRACK_STORE_TESTS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// A comp with a footage layer wearing a Planar track, a second layer to pin,
/// and a **written-down** planar track published under the effect's own id.
///
/// Written down rather than computed for [`a_tracked_layer`]'s reason:
/// `lumit-render`'s tests drive a real analysis over a rendered plane, and
/// repeating it here would be measuring the tracker again instead of the seam.
/// The quad slides ten pixels right per frame, so a reading that lands on the
/// wrong frame cannot pass by accident.
///
/// The guard it hands back is the process-wide solve store: hold it for the
/// length of the test, or another thread's `close` will empty the track this
/// one just published.
fn a_planar_tracked_layer() -> (
    crate::api::project::ProjectReference,
    CompositionReference,
    LayerReference,
    LayerReference,
    Uuid,
    std::sync::MutexGuard<'static, ()>,
) {
    let guard = track_store_test();
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project
        .new_composition(
            "Scene".into(),
            Some(BridgeCompSettings {
                name: "Scene".into(),
                width: 1920,
                height: 1080,
                fps_num: 25,
                fps_den: 1,
                duration: BridgeRational { num: 2, den: 1 },
                background: [0.0, 0.0, 0.0, 1.0],
                shutter_angle: 180.0,
                motion_blur_samples: 16,
            }),
        )
        .expect("comp");
    let footage = project
        .import_footage("C:/clips/planar.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false, None)
        .expect("placed");
    comp.add_solid_layer(None).expect("a layer to pin");
    let layers = comp.get_layers().expect("layers");
    // Newest first: the solid was added last, so it is index 0.
    let (target, shot) = (layers[0], layers[1]);
    shot.add_effect(lumit_core::track::PLANAR_TRACK.to_owned())
        .expect("the Planar track is a builtin");
    let effect = shot.get_effects().expect("the stack")[0].id();

    // Point the Pin layer row at the solid, through the staging path every
    // other effect parameter is written through.
    let mut staged = shot.get_effects().expect("the stack");
    staged[0]
        .set_value(
            lumit_core::track::PIN_LAYER_PARAM.to_owned(),
            crate::api::effect::BridgeEffectValue::Layer(Some(target.layer_id)),
        )
        .expect("the Pin layer row takes a layer");
    shot.set_effects(staged, None).expect("committed");

    let frames: Vec<lumit_track::PlanarFrame> = (0..50)
        .map(|frame| {
            let d = frame as f64 * 10.0;
            lumit_track::PlanarFrame {
                frame,
                corners: [
                    [100.0 + d, 200.0],
                    [300.0 + d, 200.0],
                    [100.0 + d, 400.0],
                    [300.0 + d, 400.0],
                ],
                inliers: 42,
                reanchored: false,
            }
        })
        .collect();
    lumit_render::track::publish_planar(
        effect,
        25.0,
        50,
        lumit_track::PlanarTrack {
            reference_frame: 0,
            reference_quad: frames[0].corners,
            frames,
            reanchors: 0,
        },
    );
    (project, comp, shot, target, effect, guard)
}

/// The status row's whole reading, and the gesture the effect exists for.
#[test]
fn a_planar_track_reports_its_span_and_writes_a_corner_pin() {
    use crate::api::track::{create_corner_pin, planar_status, BridgeTrackStage};

    let (_project, _comp, shot, target, effect, _solves) = a_planar_tracked_layer();

    let status = planar_status(shot, effect);
    assert_eq!(status.stage, BridgeTrackStage::Done);
    assert_eq!((status.frames, status.clip_frames), (50, 50));
    assert_eq!(status.reanchors, 0, "nothing was re-anchored");
    assert!(status.failure.is_none());

    // An instance nobody tracked reads Idle rather than borrowing this one's
    // answer — which is the failure a media-keyed store would have made.
    assert_eq!(
        planar_status(shot, Uuid::now_v7()).stage,
        BridgeTrackStage::Idle
    );

    create_corner_pin(shot, effect).expect("a track and a pin layer");
    let pinned = target.get_effects().expect("the target's stack");
    assert_eq!(pinned.len(), 1, "one Corner pin was appended");
    let keys = match pinned[0].get_value("upper_left_x".into()).unwrap() {
        crate::api::effect::BridgeEffectValue::Float(
            crate::api::effect::BridgeScalar::Keyframed(keys),
        ) => keys,
        other => panic!("upper left x is {other:?}, not a keyframed channel"),
    };
    assert_eq!(keys.len(), 50, "one key per comp frame of the target layer");
    assert!(
        (keys[0].value - 100.0).abs() < 1e-6,
        "the pin's first key is the track's first corner, got {}",
        keys[0].value
    );
    assert!(
        (keys[10].value - 200.0).abs() < 1e-6,
        "frame ten slid a hundred pixels, got {}",
        keys[10].value
    );

    // One undo step takes the whole pin back.
    let project = shot.project().expect("the project");
    project
        .read()
        .expect("read")
        .store
        .undo()
        .expect("the pin is one step");
    assert!(target.get_effects().expect("the target's stack").is_empty());
}

/// The other half of docs/08 §4's Tracker row: the same track pressed
/// onto the target layer's own transform, through the one doorway a press
/// crosses.
#[test]
fn transform_keys_move_the_target_layer_and_obey_the_transform_row() {
    use crate::api::effect::{BridgeEffectValue, BridgeScalar};
    use crate::api::track::fire_effect_action;

    let (_project, _comp, shot, target, effect, _solves) = a_planar_tracked_layer();

    fire_effect_action(shot, effect, "transform_keys".into(), None)
        .expect("a track and a target layer");
    let transform = target.get_transform().expect("the target's transform");

    let keys = match transform.position_x {
        BridgeScalar::Keyframed(keys) => keys,
        other => panic!("position x is {other:?}, not a keyframed channel"),
    };
    assert_eq!(keys.len(), 50, "one key per comp frame of the target layer");
    // The quad slides ten pixels a frame and the layer keeps where it was, so
    // frame ten is a hundred pixels along from frame nought — added, not
    // stamped.
    assert!(
        (keys[10].value - keys[0].value - 100.0).abs() < 1e-6,
        "frame ten should sit a hundred pixels along, got {}",
        keys[10].value - keys[0].value
    );
    // The whole movement was asked for, so the other three were written too —
    // a pure slide, so they hold what the layer had.
    assert!(matches!(transform.rotation, BridgeScalar::Keyframed(_)));
    assert!(matches!(transform.scale_x, BridgeScalar::Keyframed(_)));

    // One undo step takes the whole transform back.
    let project = shot.project().expect("the project");
    project
        .read()
        .expect("read")
        .store
        .undo()
        .expect("the keys are one step");
    let back = target.get_transform().expect("the target's transform");
    assert!(matches!(back.position_x, BridgeScalar::Static(_)));

    // Following one point: it can only say where it went, so two properties are
    // written and the rest are left as they were.
    let mut staged = shot.get_effects().expect("the stack");
    staged[0]
        .set_value(
            lumit_core::track::FOLLOW_PARAM.to_owned(),
            BridgeEffectValue::Choice(lumit_core::fx::effects::planar_track::FOLLOW_ONE_POINT),
        )
        .expect("the Follow row takes a choice");
    shot.set_effects(staged, None).expect("committed");

    fire_effect_action(shot, effect, "transform_keys".into(), None)
        .expect("position alone still writes");
    let narrow = target.get_transform().expect("the target's transform");
    assert!(matches!(narrow.position_x, BridgeScalar::Keyframed(_)));
    assert!(matches!(narrow.position_y, BridgeScalar::Keyframed(_)));
    assert!(
        matches!(narrow.rotation, BridgeScalar::Static(_)),
        "rotation should not have been written"
    );
    assert!(
        matches!(narrow.scale_x, BridgeScalar::Static(_)),
        "scale should not have been written"
    );
}

/// **Saving does not hold the project across the disk** (docs/14 §1, FP3.5).
///
/// `save` used to take the write guard first and keep it over
/// `rebase_for_save`, the serialise and the fsync — seconds of work on a large
/// project, with every `#[frb(sync)]` question the interface asks queued behind
/// it, because a Rust `RwLock` turns new readers away once a writer is waiting.
/// It now takes the destination, the revision and an `Arc` of the document
/// under a *read* guard, drops it, and writes with nothing held — the shape
/// `autosave::sweep_one` already had.
///
/// The proof is a reader that does not let go: this test holds a read guard for
/// the whole of another thread's save, and the file has to appear on disk
/// anyway. Under the old code nothing could be written until the guard was
/// released.
#[test]
fn saving_writes_the_file_while_a_reader_holds_the_project() {
    // `close` empties the process-wide solve store, so this waits for the
    // planar tests rather than emptying one mid-read.
    let _solves = track_store_test();
    let (project, ..) = project_with_folder();
    let dir = std::env::temp_dir().join("lumit-save-lock-freedom");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    let target = dir.join("held.lum");

    let state = project.state().expect("state");
    let reader = state.read().expect("a reader gets in");

    let saving = {
        let project = project.clone();
        let target = target.clone();
        std::thread::spawn(move || project.save(target.to_string_lossy().into_owned()))
    };

    // The save's own write guard comes back at the end, to record where the
    // file went — so the file lands first, and this waits for it rather than
    // for the thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !target.is_file() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        target.is_file(),
        "the document was serialised and written with a reader still holding \
         the project: a guard held across the disk is the interface stopped"
    );

    drop(reader);
    let written = saving.join().expect("the saving thread").expect("saved");
    assert!(written.ends_with("held.lum"));
    assert!(
        !project.is_dirty().expect("dirty"),
        "the revision the file contains is the one stamped as saved"
    );

    project.close().expect("closed");
    std::fs::remove_dir_all(&dir).ok();
}

/// **A thumbnail is answered without the write guard** (docs/14 §1, FP3.5).
///
/// `thumbnail` used to take the *write* half of the project's lock and hold it
/// across an FFmpeg decode — tens of milliseconds on a cold file, with every
/// reader turned away for the whole of it. It now reads the path and any
/// cached picture under a read guard, decodes with nothing held, and takes the
/// lock again only to store the result.
///
/// A reader that does not let go is the proof again: with the picture already
/// decoded there is nothing left to store, so the answer must come back while
/// this test holds a read guard. Under the old code the write guard could not
/// be had and the call would not return at all.
///
/// Needs an ffmpeg on PATH for the fixture; skips itself without one.
#[test]
#[cfg(feature = "media")]
fn a_cached_thumbnail_is_answered_while_a_reader_holds_the_project() {
    // `close` empties the process-wide solve store, so this waits for the
    // planar tests rather than emptying one mid-read.
    let _solves = track_store_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let Some(clip) = lumit_media::index::tests_support::fixture(dir.path()) else {
        return; // no ffmpeg on this machine
    };

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let footage = project
        .import_footage(clip.to_string_lossy().into_owned())
        .expect("imported");
    let first = footage
        .thumbnail(32, 0)
        .expect("decoded")
        .expect("a picture");

    let state = project.state().expect("state");
    let reader = state.read().expect("a reader gets in");

    // Through a channel rather than a join, so a regression fails the test
    // instead of hanging it: under the old code the answer never comes.
    let (tell, answer) = std::sync::mpsc::channel();
    let held = std::thread::spawn({
        let (p, id) = (footage.project_id(), footage.id());
        move || tell.send(FootageReference::new(p, id).thumbnail(32, 0))
    });
    let again = answer
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("answered with a reader still holding the project")
        .expect("decoded")
        .expect("a picture");

    drop(reader);
    held.join().expect("the thumbnail thread").expect("sent");
    assert_eq!(
        (first.width, first.height),
        (again.width, again.height),
        "the same picture, from the cache"
    );

    project.close().expect("closed");
}

// ----------------------------------------------------------------- plugins --

/// The test plugin's file name on this platform.
fn ofx_test_plugin_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_ofx_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_ofx_testplug.dylib"
    } else {
        "liblumit_ofx_testplug.so"
    }
}

/// Lay the test plugin out as a bundle in `root`, or `None` if Cargo has not
/// built it — in which case the caller says so and does nothing.
fn an_ofx_bundle_in(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = ofx_test_plugin_file_name();
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    let source = loop {
        if dir.join(name).is_file() {
            break dir.join(name);
        }
        if dir.join("deps").join(name).is_file() {
            break dir.join("deps").join(name);
        }
        dir = dir.parent()?;
    };
    let into = root
        .join("Bridge.ofx.bundle")
        .join("Contents")
        .join(lumit_ofx::bundle::BUNDLE_ARCH_DIR);
    std::fs::create_dir_all(&into).ok()?;
    let binary = into.join("bridge.ofx");
    std::fs::copy(&source, &binary).ok()?;
    Some(binary)
}

/// A discovered plugin reaches the browser as an ordinary listing entry — under
/// its **own** grouping, saying where it came from, and with its parameters
/// available to Effect controls without a second kind of lookup (docs/12 §2.6).
///
/// The scan is in-process: whether a plugin becomes a catalogue entry the bridge
/// can list is the question here, and a broker process would only add a way for
/// the test to be flaky.
#[test]
fn a_discovered_plugin_lists_under_its_own_grouping_with_its_provenance() {
    use crate::api::effect::{list_effects, list_parameters, NAMESPACE_OFX};

    let root = tempfile::tempdir().expect("a temp dir");
    let Some(_binary) = an_ofx_bundle_in(root.path()) else {
        eprintln!(
            "a_discovered_plugin_lists_under_its_own_grouping_with_its_provenance: skipped — {} \
             was not built. cargo build -p lumit-ofx-testplug",
            ofx_test_plugin_file_name()
        );
        return;
    };

    let options = lumit_ofx::discover::ScanOptions {
        paths: vec![root.path().to_path_buf()],
        disabled: std::collections::BTreeSet::new(),
        hosting: lumit_ofx::discover::Hosting::InProcess,
    };
    // The real registration, into both tables at once — this is exactly what
    // `rescan_plugins` does, minus reading the user's preferences off disk.
    lumit_ofx::discover::scan(&options, &mut lumit_render::gpufx::ofx::register);

    let listed = list_effects();
    let plugin = listed
        .iter()
        .find(|e| e.name == "ofx:com.lumitlab.testplug")
        .expect("the scanned plugin is in the one listing the browser reads");
    assert_eq!(plugin.label, "Test plug");
    assert_eq!(
        plugin.namespace, NAMESPACE_OFX,
        "the provenance rides on the listing, so the context menu needs no second call"
    );
    assert_eq!(
        (plugin.category.as_str(), plugin.category_label.as_str()),
        ("ofx/Lumit/Test", "Lumit/Test"),
        "a plugin is placed under its own declared grouping, not under one of ours"
    );
    assert!(
        listed
            .iter()
            .any(|e| e.name == "blur" && e.namespace == crate::api::effect::NAMESPACE_BUILTIN),
        "and the built-ins are still all there, still saying they are ours"
    );

    // Its parameters reach Effect controls through the same call a built-in's
    // do — the four schema lookups now walk the whole catalogue.
    let params = list_parameters(plugin.name.clone());
    assert!(
        params.iter().any(|p| p.id == "gain"),
        "a plugin's own rows are listed: {:?}",
        params.iter().map(|p| &p.id).collect::<Vec<_>>()
    );
    assert!(
        crate::api::effect::list_parameter_groups(plugin.name.clone())
            .iter()
            .any(|g| g.label == "Advanced"),
        "and so is the layout it declared"
    );

    // A rescan is not a second catalogue.
    let again = lumit_ofx::discover::scan(&options, &mut lumit_render::gpufx::ofx::register);
    assert!(again.registered.is_empty());
    assert_eq!(
        list_effects()
            .iter()
            .filter(|e| e.name == "ofx:com.lumitlab.testplug")
            .count(),
        1
    );
}

/// An effect this build has never heard of stays an inert placeholder with a
/// calm badge, and the badge says **which** kind of nothing it is: a plugin the
/// machine has not got, or an effect from a newer Lumit (docs/12 §1).
#[test]
fn an_unknown_effect_is_a_badged_placeholder_and_never_an_error() {
    use lumit_core::model::{EffectInstance, EffectKey, EffectNamespace};

    let instance = |namespace, match_name: &str| EffectInstance {
        id: Uuid::now_v7(),
        effect: EffectKey {
            namespace,
            match_name: match_name.to_owned(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        roto: None,
        enabled: true,
        params: Vec::new(),
        sample_temporally: true,
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
        extra: serde_json::Map::new(),
    };

    let missing = crate::api::effect::read_instance_info(
        &instance(EffectNamespace::Ofx, "ofx:com.nobody.notinstalled"),
        lumit_core::time::Rational::ZERO,
    );
    assert_eq!(missing.badge_reason.as_deref(), Some("plugin_missing"));
    assert_eq!(missing.badge_detail, None);
    assert_eq!(
        missing.name, "ofx:com.nobody.notinstalled",
        "the instance is kept exactly as it was, so saving cannot lose it"
    );

    // An audio plugin is the same story whichever standard minted the name:
    // both prefixes land in one namespace, and a machine without the plugin
    // badges it as missing rather than as an effect from the future.
    for name in [
        "clap:com.nobody.notinstalled",
        "vst3:0123456789abcdef0123456789abcdef",
    ] {
        let absent = crate::api::effect::read_instance_info(
            &instance(EffectNamespace::Clap, name),
            lumit_core::time::Rational::ZERO,
        );
        assert_eq!(
            absent.badge_reason.as_deref(),
            Some("plugin_missing"),
            "{name} is a plugin this machine has not got"
        );
        assert_eq!(
            absent.name, name,
            "and the instance is kept exactly as it was"
        );
        assert_eq!(
            lumit_core::fx::instantiate(name),
            None,
            "a name nothing answers to instantiates nothing, rather than              borrowing some other effect's declaration"
        );
    }

    let stranger = crate::api::effect::read_instance_info(
        &instance(EffectNamespace::Placeholder, "from_a_newer_lumit"),
        lumit_core::time::Rational::ZERO,
    );
    assert_eq!(stranger.badge_reason.as_deref(), Some("unknown_effect"));

    // And a built-in wears no badge at all.
    let blur = lumit_core::fx::instantiate("blur").expect("Gaussian blur is built in");
    let ordinary = crate::api::effect::read_instance_info(&blur, lumit_core::time::Rational::ZERO);
    assert_eq!(ordinary.badge_reason, None);

    // Every reason the engine can file has a name in the closed list the
    // frontend's translations are held against.
    for reason in [
        missing.badge_reason.as_deref(),
        stranger.badge_reason.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        assert!(
            crate::api::effect::BADGE_REASONS.contains(&reason),
            "{reason} is not in BADGE_REASONS, so nothing will translate it"
        );
    }
}

/// Pressing a plugin's button writes what the plugin did into the document,
/// as one undo step: a row it set as that row, and everything no row carries
/// as the instance's own memory. The plugin is shown the comp at the playhead
/// while it is pressed. What it did to its controls in answer is kept in the
/// same step, and the panel reads it from there.
#[test]
fn pressing_a_plugins_button_writes_what_it_did_into_the_document() {
    use std::sync::{Arc, Mutex};

    /// A host whose button writes one row and one blob, the way Looks stores a
    /// look, and remembers the frame it was shown.
    struct Pressing {
        shown: Mutex<Option<(usize, usize)>>,
    }

    impl lumit_ofx::PluginHost for Pressing {
        fn render(
            &self,
            _instance: Uuid,
            _time: f64,
            _params: &lumit_ofx::ParamSnapshot,
            source: lumit_ofx::Frame16,
            _neighbours: &[(i32, lumit_ofx::Frame16)],
        ) -> lumit_ofx::Rendering {
            lumit_ofx::Rendering {
                frame: source,
                error: None,
            }
        }

        fn frames_needed(
            &self,
            _instance: Uuid,
            _time: f64,
            _params: &lumit_ofx::ParamSnapshot,
        ) -> Option<Vec<i32>> {
            None
        }

        fn press(
            &self,
            _instance: Uuid,
            _time: f64,
            params: &lumit_ofx::ParamSnapshot,
            name: &str,
            source: lumit_ofx::Frame16,
        ) -> Result<lumit_ofx::Settled, String> {
            assert_eq!(name, "trigger", "the plugin's own name for the button");
            *self.shown.lock().expect("the record") = Some((source.width(), source.height()));
            let mut after = params.clone();
            after.set("gain", lumit_ofx::PropValue::double(0.75));
            after.set(
                "vendorBlob",
                lumit_ofx::PropValue::string("pressed").expect("a string"),
            );
            Ok(lumit_ofx::Settled {
                params: after,
                controls: lumit_ofx::Controls::default(),
            })
        }

        /// Greys its button once gain is off its default, the way a plugin
        /// greys what an edit rules out.
        fn settle(
            &self,
            _instance: Uuid,
            _made_with: &lumit_ofx::ParamSnapshot,
            handed: &lumit_ofx::ParamSnapshot,
        ) -> Result<lumit_ofx::Settled, String> {
            let mut controls = lumit_ofx::Controls::default();
            if handed.get("gain") != Some(&lumit_ofx::PropValue::double(1.0)) {
                controls.disabled.insert("trigger".to_owned());
            }
            Ok(lumit_ofx::Settled {
                params: handed.clone(),
                controls,
            })
        }
    }

    let param = |name: &str, kind: &str, default: Option<f64>| {
        let mut props = lumit_ofx::PropertySet::new();
        if let Some(default) = default {
            props.seed(
                lumit_ofx::ffi::prop_keys::PARAM_DEFAULT,
                lumit_ofx::PropValue::double(default),
            );
        }
        lumit_ofx::describe::ParamDescription {
            name: name.to_owned(),
            param_type: kind.to_owned(),
            props,
        }
    };
    let descriptor = lumit_ofx::PluginDescriptor {
        identifier: "com.lumitlab.pressing".to_owned(),
        version: (1, 0),
        grouping: "Lumit/Test".to_owned(),
        label: "Pressing".to_owned(),
        contexts: vec![lumit_ofx::Context::Filter],
        params: vec![
            param("gain", lumit_ofx::ffi::param_types::DOUBLE, Some(1.0)),
            param("vendorBlob", lumit_ofx::ffi::param_types::CUSTOM, None),
            param("trigger", lumit_ofx::ffi::param_types::PUSH_BUTTON, None),
        ],
        clips: Vec::new(),
        temporal: false,
        render_thread_safety: None,
    };
    let schema: &'static lumit_core::fx::EffectSchema = Box::leak(Box::new(
        lumit_ofx::schema_of(&descriptor).expect("a schema"),
    ));
    let host = Arc::new(Pressing {
        shown: Mutex::new(None),
    });
    let def = lumit_ofx::OfxEffectDef::new(&descriptor, schema, host.clone()).leak();
    assert!(lumit_render::gpufx::ofx::register(def));

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let target = comp.add_solid_layer(None).expect("a layer for the plugin");
    target
        .add_effect(schema.match_name.to_owned())
        .expect("the plugin added");
    let effect = target.item().expect("the layer").effects[0].id;

    crate::api::track::press_plugin_now(&target, effect, "trigger", 0)
        .expect("the press went through");

    let shown = host
        .shown
        .lock()
        .expect("the record")
        .expect("the plugin was shown a frame");
    assert!(
        shown.0 > 0 && shown.1 > 0,
        "a real frame, not an empty one: {shown:?}"
    );

    let fx = target.item().expect("the layer").effects[0].clone();
    let gain = fx
        .params
        .iter()
        .find(|p| p.id == "gain")
        .expect("the row the plugin wrote");
    match &gain.value {
        lumit_core::model::EffectValue::Float(property) => {
            assert!((property.value_at(0.0) - 0.75).abs() < 1e-9, "{property:?}");
        }
        other => panic!("gain is a float row, got {other:?}"),
    }
    assert!(
        fx.plugin_state_bytes()
            .is_some_and(|bytes| !bytes.is_empty()),
        "the blob has no row, so it is the instance's memory"
    );
    assert_eq!(
        def.row_state(&fx).disabled,
        ["trigger"],
        "the plugin answered the edit, and the document kept what it greyed"
    );

    // One undo step takes both back.
    project.undo().expect("undone");
    let fx = target.item().expect("the layer").effects[0].clone();
    let gain = fx.params.iter().find(|p| p.id == "gain").expect("the row");
    match &gain.value {
        lumit_core::model::EffectValue::Float(property) => {
            assert!((property.value_at(0.0) - 1.0).abs() < 1e-9, "{property:?}");
        }
        other => panic!("gain is a float row, got {other:?}"),
    }
    assert_eq!(fx.plugin_state, None);

    // A button the plugin has not got is refused, and the layer wears the
    // plugin's own sentence.
    assert!(crate::api::track::press_plugin_now(&target, effect, "nothing", 0).is_err());
    assert!(lumit_render::gpufx::ofx::error_of(effect).is_some());
    lumit_render::gpufx::ofx::clear_errored(effect);
}

/// Register one stand-in definition under `name`, in `category`. That is the
/// shape the scan registers for an audio plugin, minus the broker, so these
/// tests need no plugin installed, and the shape a built-in audio effect
/// declares.
/// Registration is by name and additive, so each test takes names of its own.
fn register_audio_def(name: &'static str, label: &'static str) {
    register_def_in(name, label, lumit_core::fx::FxCategory::Utility);
}

fn register_def_in(name: &'static str, label: &'static str, category: lumit_core::fx::FxCategory) {
    struct AudioDef(&'static lumit_core::fx::EffectSchema);
    impl lumit_core::fx::EffectDef for AudioDef {
        fn schema(&self) -> &'static lumit_core::fx::EffectSchema {
            self.0
        }
        fn is_image_op(&self) -> bool {
            false
        }
    }
    let schema: &'static lumit_core::fx::EffectSchema =
        Box::leak(Box::new(lumit_core::fx::EffectSchema {
            match_name: name,
            label,
            version: 1,
            category,
            traits: lumit_core::fx::EffectTraits {
                cost: lumit_core::fx::CostClass::Heavy,
                roi: lumit_core::fx::Roi::FullFrame,
                temporal: &[0],
                premultiplied: true,
                seeded: false,
                beat_input: false,
            },
            params: Box::leak(Box::new([lumit_core::fx::ParamSchema {
                id: "p1",
                label: "Gain",
                kind: lumit_core::fx::ParamKind::Slider {
                    default: 1.0,
                    range: (0.0, 4.0),
                    log: false,
                },
                unit: lumit_core::fx::Unit::Raw,
            }])),
            groups: &[],
            enabled_when: &[],
            matte: lumit_core::fx::MatteRole::None,
        }));
    lumit_core::fx::BUILTIN_DEFS.register(Box::leak(Box::new(AudioDef(schema))));
}

/// **The browser's share of AP5, bridge side**: an audio plugin lists under
/// the one Audio plugins group with its provenance, its rows appear and take
/// a write like any effect's, and the switch-off answers at once with the
/// calm `plugin_disabled` badge on every instance — then comes off again.
#[test]
fn an_audio_plugin_lists_under_the_audio_group_and_switches_off() {
    register_audio_def("clap:com.lumitlab.aptest", "Test EQ");

    let plugin = list_effects()
        .into_iter()
        .find(|e| e.name == "clap:com.lumitlab.aptest")
        .expect("the registered audio plugin is in the one listing the browser reads");
    assert_eq!(
        plugin.namespace,
        crate::api::effect::NAMESPACE_AUDIO,
        "the provenance rides on the listing"
    );
    assert_eq!(
        plugin.category, "audio",
        "one group for every audio plugin, beside the OFX ones"
    );
    assert_eq!(
        plugin.category_label, "",
        "unheaded on purpose: the frontend words the Audio plugins heading"
    );

    // Its rows appear on an instance and take a write, exactly as a
    // built-in's do — the stack is the rack.
    let (_project, layer) = project_with_layer();
    layer
        .add_effect("clap:com.lumitlab.aptest".into())
        .expect("an audio plugin is an ordinary stack entry");
    let stack = layer.get_effects().expect("stack");
    let info = stack[0].get_info();
    assert!(
        info.values.iter().any(|v| v.id == "p1"),
        "the plugin's own row crossed with a value to draw"
    );
    assert_eq!(info.badge_reason, None, "a working plugin wears no badge");
    let mut stack = stack;
    stack[0]
        .set_value(
            "p1".to_owned(),
            BridgeEffectValue::Float(BridgeScalar::Static(2.5)),
        )
        .expect("a plugin row takes a write like any other");
    layer.set_effects(stack, None).expect("committed");
    assert_eq!(
        layer.get_effects().expect("stack")[0]
            .get_value("p1".to_owned())
            .expect("read back"),
        BridgeEffectValue::Float(BridgeScalar::Static(2.5))
    );

    // Switch it off: the session list holds the bare identifier, and every
    // instance wears the calm badge at once — the layer keeps its rows.
    let _ = crate::api::effect::set_plugin_enabled("clap:com.lumitlab.aptest".to_owned(), false);
    assert!(lumit_aplug::session_disabled()
        .lock()
        .expect("list")
        .contains("com.lumitlab.aptest"));
    let off = crate::api::effect::read_instance_info(
        &stack_of(&layer)[0],
        lumit_core::time::Rational::ZERO,
    );
    assert_eq!(off.badge_reason.as_deref(), Some("plugin_disabled"));
    assert_eq!(off.badge_detail, None);

    let _ = crate::api::effect::set_plugin_enabled("clap:com.lumitlab.aptest".to_owned(), true);
    let back = crate::api::effect::read_instance_info(
        &stack_of(&layer)[0],
        lumit_core::time::Rational::ZERO,
    );
    assert_eq!(back.badge_reason, None, "switching back on lifts the badge");
}

/// The badge path, end to end through the bridge: a plugin whose render fails
/// hands back its input unchanged, the comp carries on, and **the layer wears a
/// badge with the plugin's own sentence under it** (docs/12 §2.3, the Gate-4
/// demo in docs/16 line 85). The next frame that works takes the badge off
/// again.
///
/// The failure here is filed by the definition, not fabricated by the test: a
/// host that answers "the plugin stopped answering" is what a crashed broker,
/// a missed deadline and a plugin switched off mid-session all look like from
/// this side of the seam (docs/impl/ofx-host.md §5 item 4). That a *crash*
/// really does end in that answer, in another process, is the broker crate's
/// own test — it needs a second process and this needs none.
#[test]
fn a_plugin_that_fails_a_frame_badges_its_layer_and_the_next_frame_clears_it() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A host that fails on demand. Failing means: the picture goes back
    /// untouched and a sentence comes with it.
    struct Moody {
        failing: AtomicBool,
        why: String,
    }

    impl lumit_ofx::PluginHost for Moody {
        fn render(
            &self,
            _instance: Uuid,
            _time: f64,
            _params: &lumit_ofx::ParamSnapshot,
            source: lumit_ofx::Frame16,
            _neighbours: &[(i32, lumit_ofx::Frame16)],
        ) -> lumit_ofx::Rendering {
            if self.failing.load(Ordering::SeqCst) {
                return lumit_ofx::Rendering {
                    frame: source,
                    error: Some(self.why.clone()),
                };
            }
            lumit_ofx::Rendering {
                frame: source,
                error: None,
            }
        }

        fn frames_needed(
            &self,
            _instance: Uuid,
            _time: f64,
            _params: &lumit_ofx::ParamSnapshot,
        ) -> Option<Vec<i32>> {
            None
        }

        fn press(
            &self,
            _instance: Uuid,
            _time: f64,
            _params: &lumit_ofx::ParamSnapshot,
            _name: &str,
            _source: lumit_ofx::Frame16,
        ) -> Result<lumit_ofx::Settled, String> {
            Err(self.why.clone())
        }
    }

    /// Register one plugin definition backed by `host`, and hand back the
    /// definition and the name it answers to.
    fn a_registered_plugin(
        identifier: &str,
        host: Arc<dyn lumit_ofx::PluginHost>,
    ) -> (&'static dyn lumit_core::fx::EffectDef, String) {
        let descriptor = lumit_ofx::PluginDescriptor {
            identifier: identifier.to_owned(),
            version: (1, 0),
            grouping: "Lumit/Test".to_owned(),
            label: "Moody".to_owned(),
            contexts: vec![lumit_ofx::Context::Filter],
            params: Vec::new(),
            clips: Vec::new(),
            temporal: false,
            render_thread_safety: None,
        };
        let schema: &'static lumit_core::fx::EffectSchema = Box::leak(Box::new(
            lumit_ofx::schema_of(&descriptor).expect("a plugin with no parameters has a schema"),
        ));
        let name = schema.match_name.to_owned();
        let def = lumit_ofx::OfxEffectDef::new(&descriptor, schema, host).leak();
        assert!(
            lumit_render::gpufx::ofx::register(def),
            "the catalogue and the pass table both took {name}"
        );
        (def, name)
    }

    let failed_why = "the plugin stopped answering";
    let moody = Arc::new(Moody {
        failing: AtomicBool::new(true),
        why: failed_why.to_owned(),
    });
    let (def, name) = a_registered_plugin("com.lumitlab.moody", moody.clone());

    let instance = lumit_core::fx::instantiate(&name).expect("the plugin is in the catalogue");
    let mut rgba = vec![0.25_f32; 4 * 4 * 4];
    let before = rgba.clone();
    lumit_render::gpufx::ofx::apply_and_note(
        def,
        instance.id,
        0.0,
        &mut rgba,
        4,
        4,
        lumit_core::fx::Params::EMPTY,
        &[],
    );

    // **Identity, byte for byte.** A failed plugin costs the layer its effect,
    // never its picture.
    assert_eq!(
        rgba, before,
        "a failed render left the frame exactly as it was"
    );

    let badged =
        crate::api::effect::read_instance_info(&instance, lumit_core::time::Rational::ZERO);
    assert_eq!(badged.badge_reason.as_deref(), Some("plugin_failed"));
    assert_eq!(
        badged.badge_detail.as_deref(),
        Some(failed_why),
        "the plugin's own words are shown verbatim beneath the badge"
    );
    assert!(
        crate::api::effect::BADGE_REASONS.contains(&"plugin_failed"),
        "the reason is a key the frontend can translate"
    );

    // The session carries on: the next frame works, and the badge goes.
    moody.failing.store(false, Ordering::SeqCst);
    lumit_render::gpufx::ofx::apply_and_note(
        def,
        instance.id,
        1.0,
        &mut rgba,
        4,
        4,
        lumit_core::fx::Params::EMPTY,
        &[],
    );
    let cleared =
        crate::api::effect::read_instance_info(&instance, lumit_core::time::Rational::ZERO);
    assert_eq!(cleared.badge_reason, None);
    assert_eq!(cleared.badge_detail, None);

    // A plugin switched off files the reason as a **key**, and the badge says
    // "switched off" rather than reporting somebody's failure.
    let disabled = Arc::new(Moody {
        failing: AtomicBool::new(true),
        why: lumit_ofx::DISABLED_REASON.to_owned(),
    });
    let (off_def, off_name) = a_registered_plugin("com.lumitlab.moody.off", disabled);
    let off = lumit_core::fx::instantiate(&off_name).expect("the plugin is in the catalogue");
    lumit_render::gpufx::ofx::apply_and_note(
        off_def,
        off.id,
        0.0,
        &mut rgba,
        4,
        4,
        lumit_core::fx::Params::EMPTY,
        &[],
    );
    let switched_off =
        crate::api::effect::read_instance_info(&off, lumit_core::time::Rational::ZERO);
    assert_eq!(
        switched_off.badge_reason.as_deref(),
        Some("plugin_disabled")
    );
    assert_eq!(
        switched_off.badge_detail, None,
        "a key is never shown to a person as a sentence"
    );

    lumit_render::gpufx::ofx::clear_errored(instance.id);
    lumit_render::gpufx::ofx::clear_errored(off.id);
}

// ---------------------------------------------------------------------------
// The Custom shader across the seam (docs/impl/custom-shader.md CS2).
// ---------------------------------------------------------------------------

/// A shader with two annotated uniforms, one of each shape the panel draws
/// differently: a slider in pixels and a colour.
const TWO_ROWS: &str = "\
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Radius
    radius: f32,
    /// @colour @default(1, 0.5, 0.2, 1) Tint
    tint: vec4<f32>,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.tint * p.radius;
}
";

/// Stage `source` on the layer's one effect and commit it — the panel's own
/// road: one handle, one `set_effects`, one op.
fn set_shader(layer: &LayerReference, source: &str, origin: Option<&str>) {
    let stack = layer.get_effects().expect("stack");
    let mut stack = stack;
    stack[0].set_shader_source(source.to_owned(), origin.map(str::to_owned));
    layer.set_effects(stack, None).expect("committed");
}

fn only_effect(layer: &LayerReference) -> BridgeEffectInstance {
    layer
        .get_effects()
        .expect("stack")
        .into_iter()
        .next()
        .expect("one effect")
}

/// Three rows: two floats to key and drive, and a colour.
const THREE_ROWS: &str = "\
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Radius
    radius: f32,
    /// @slider(0, 1) Wobble
    wobble: f32,
    /// @colour @default(1, 0.5, 0.2, 1) Tint
    tint: vec4<f32>,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.tint * p.radius * p.wobble;
}
";

/// The edit after it: the two floats are gone and a whole number is new.
const NEXT_ROWS: &str = "\
struct Params {
    /// @colour @default(1, 0.5, 0.2, 1) Tint
    tint: vec4<f32>,
    /// Steps
    steps: i32,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.tint * f32(p.steps);
}
";

/// **Adopting and removing are the user's acts, and each says what it does**
/// (docs/impl/effect-registry.md §4 rules 1 and 2, on the effect they were
/// written for).
///
/// A shader edit offers rows and adopts none; Sync adopts them in one commit;
/// an edit that stops naming a row leaves it, keyframes and expression intact,
/// and says what removing it would cost; Remove is the one road that takes it
/// away, and undo is the way back.
#[test]
fn sync_adopts_the_offered_rows_and_remove_takes_the_unused_ones() {
    let (project, layer) = project_with_layer();
    layer.add_effect("custom_shader".into()).expect("added");
    set_shader(&layer, THREE_ROWS, None);

    // Apply wrote the text and nothing else: the rows are offered, the
    // document does not hold them, and every row says which half it is from.
    let fresh = only_effect(&layer);
    let sync = fresh.parameter_sync();
    assert_eq!(
        sync.adds,
        ["radius", "wobble", "tint"],
        "offered, not adopted"
    );
    assert!(sync.removes.is_empty());
    let rows = fresh.list_parameters();
    assert_eq!(
        rows.iter()
            .filter(|r| r.derived)
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["radius", "wobble", "tint"],
        "the derived tail carries the flag"
    );
    assert!(
        rows.iter().any(|r| r.id == "mix" && !r.derived),
        "the declared head does not"
    );
    assert!(fresh.get_info().derived_params.iter().all(|r| r.derived));
    assert!(
        crate::api::effect::list_parameters("custom_shader".into())
            .iter()
            .all(|r| !r.derived),
        "a match name can only ever answer the declared half"
    );
    assert_eq!(
        only_effect(&layer).parameter_sync().adds,
        ["radius", "wobble", "tint"],
        "reading the stack adopts nothing"
    );

    // Sync: one staged act, one commit.
    let mut stack = layer.get_effects().expect("stack");
    assert_eq!(stack[0].sync_parameters(), ["radius", "wobble", "tint"]);
    assert!(
        stack[0].sync_parameters().is_empty(),
        "a second press has nothing left to adopt"
    );
    layer.set_effects(stack, None).expect("committed");
    assert!(
        only_effect(&layer).parameter_sync().adds.is_empty(),
        "the document holds them now"
    );

    // Key one adopted row and drive another, exactly as declared rows are.
    let key = |num: i64, value: f64| BridgeKeyframe {
        time: BridgeRational { num, den: 1 },
        value,
        interp_in: BridgeSideInterp::Linear,
        interp_out: BridgeSideInterp::Linear,
    };
    let mut stack = layer.get_effects().expect("stack");
    stack[0]
        .set_value(
            "radius".into(),
            BridgeEffectValue::Float(BridgeScalar::Keyframed(vec![key(0, 10.0), key(2, 90.0)])),
        )
        .expect("keyed");
    stack[0]
        .set_value(
            "wobble".into(),
            BridgeEffectValue::Float(BridgeScalar::Expression(
                "time".into(),
                crate::api::effect::BridgeExpressionLanguage::Rhai,
            )),
        )
        .expect("driven");
    layer.set_effects(stack, None).expect("committed");

    // The edit that stops naming them: rule 1, and the cost of removal.
    set_shader(&layer, NEXT_ROWS, None);
    let edited = only_effect(&layer);
    let sync = edited.parameter_sync();
    assert_eq!(sync.adds, ["steps"], "the new row is offered");
    assert_eq!(
        sync.removes
            .iter()
            .map(|r| (r.id.as_str(), r.keyframed, r.expression))
            .collect::<Vec<_>>(),
        [("radius", true, false), ("wobble", false, true)],
        "the rows nothing names any more, and what each one holds"
    );
    assert!(
        matches!(
            edited.get_value("radius".into()),
            Ok(BridgeEffectValue::Float(BridgeScalar::Keyframed(keys))) if keys.len() == 2
        ),
        "the keys are still there"
    );
    assert!(
        edited.get_value("steps".into()).is_ok(),
        "the offered row is live on the copy"
    );

    // Remove: says what it took, one commit, and undo is the way back.
    let mut stack = layer.get_effects().expect("stack");
    assert_eq!(stack[0].remove_unused_parameters(), ["radius", "wobble"]);
    layer.set_effects(stack, None).expect("committed");
    let trimmed = only_effect(&layer);
    assert!(trimmed.parameter_sync().removes.is_empty());
    assert!(
        trimmed.get_value("radius".into()).is_err(),
        "gone from the document"
    );
    assert_eq!(
        trimmed.parameter_sync().adds,
        ["steps"],
        "removing adopts nothing"
    );
    project.undo().expect("undone");
    assert!(
        matches!(
            only_effect(&layer).get_value("radius".into()),
            Ok(BridgeEffectValue::Float(BridgeScalar::Keyframed(keys))) if keys.len() == 2
        ),
        "one undo puts the keyed row back"
    );

    // A source staged on a handle drops the offers made for the one before
    // it, so an Apply never writes a row nobody asked for.
    let mut stack = layer.get_effects().expect("stack");
    assert!(stack[0].get_value("steps".into()).is_ok(), "offered again");
    stack[0].set_shader_source(THREE_ROWS.into(), None);
    assert!(
        stack[0].get_value("steps".into()).is_err(),
        "the new text does not name it, and it was never the document's"
    );
    assert!(stack[0].parameter_sync().adds.is_empty());
    layer.set_effects(stack, None).expect("committed");
    assert!(only_effect(&layer).get_value("steps".into()).is_err());
}

/// **The compile state crosses in both directions**, and a refusal and a
/// compile error read as one sentence because the person is looking at one text
/// box (§2.1, §2.2).
#[test]
fn a_shaders_status_answers_both_states() {
    let (_project, layer) = project_with_layer();
    layer.add_effect("custom_shader".into()).expect("added");

    set_shader(&layer, TWO_ROWS, Some("C:/shaders/warp.wgsl"));
    let good = only_effect(&layer);
    assert_eq!(good.shader_status().error, None, "this one compiles");
    assert_eq!(good.get_info().badge_reason, None, "and wears no badge");
    assert_eq!(good.shader_source().as_deref(), Some(TWO_ROWS));
    assert_eq!(
        good.shader_origin().as_deref(),
        Some("C:/shaders/warp.wgsl"),
        "where it came from is remembered for reload"
    );

    // A refusal: the one thing the contract asks for is missing, and the reader
    // says so without a graphics card anywhere near it.
    set_shader(
        &layer,
        "fn other(uv: vec2<f32>) -> f32 { return 0.0; }",
        None,
    );
    let refused = only_effect(&layer);
    let why = refused.shader_status().error.expect("refused");
    assert!(
        why.contains("shade"),
        "the refusal names the contract: {why}"
    );

    // A compile error: valid enough to assemble, wrong once naga reads it. The
    // line number is the user's own — line 3 of what they typed, not line 3 of
    // the wrapper Lumit put around it.
    set_shader(
        &layer,
        "fn shade(uv: vec2<f32>) -> vec4<f32> {\n    \
         let a = 1.0;\n    \
         return nonesuch(uv);\n}\n",
        None,
    );
    let broken = only_effect(&layer);
    let message = broken.shader_status().error.expect("it does not compile");
    assert!(
        message.contains("wgsl:3:"),
        "the compiler's line numbers are remapped onto the user's text: {message}"
    );
    let info = broken.get_info();
    assert_eq!(
        info.badge_reason.as_deref(),
        Some("shader_failed"),
        "a shader that will not compile wears the calm badge, never an alarm"
    );
    assert_eq!(
        info.badge_detail,
        Some(message),
        "the compiler's own sentence goes underneath, untranslated"
    );

    // Emptied: back to a passthrough, and a passthrough is not a failure.
    set_shader(&layer, "", None);
    let cleared = only_effect(&layer);
    assert_eq!(cleared.shader_source(), None);
    assert_eq!(cleared.shader_status().error, None);
    assert_eq!(cleared.get_info().badge_reason, None);
}

// --- The project's own picture, off the Viewer ----------------------------

/// **The road a headless save takes.** The welcome screen's thumbnail used to be
/// a photograph of the Viewer widget, so a project saved with no Viewer on
/// screen — an After Effects conversion, a script, an autosave with the panel
/// closed — got no picture and the row showed an empty well. Nothing here starts
/// a worker or mounts anything: a project, a composition, a layer, and a still.
///
/// `max_edge` is honoured on the *longest* edge, so a 16:9 comp answers 128×72
/// and the row's 64×36 well is filled at 200 % and no more.
///
/// Skips itself calmly on a machine with no graphics adapter, the way every
/// other test that needs one does — and that is the same `None` a row treats as
/// "no picture yet", not as an error.
#[test]
fn a_composition_draws_its_own_thumbnail_without_a_viewer() {
    // `close` empties the process-wide solve store, so this waits for the
    // planar tests rather than emptying one mid-read.
    let _solves = track_store_test();
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = add_comp(&project, "Scene");
    comp.add_solid_layer(None).expect("something to draw");

    let Some(thumb) = comp.thumbnail(0, 128).expect("the comp is a comp") else {
        eprintln!("no graphics adapter; skipping");
        return;
    };

    assert_eq!(
        (thumb.width, thumb.height),
        (128, 72),
        "the longest edge is what `max_edge` names, and the shape is the comp's"
    );
    assert_eq!(
        thumb.rgba.len(),
        128 * 72 * 4,
        "tightly packed RGBA8, as the type promises"
    );
    assert!(
        thumb.rgba.iter().any(|&byte| byte != 0),
        "a solid on a comp is not an empty picture"
    );

    project.close().expect("closed");
}

// ---------------------------------------------------------------------------
// The node graph composition (docs/impl/node-graph-comp.md §4.1).

use crate::api::comp_graph::{
    BridgeCompEdge, BridgeCompNodeKind, BridgeCompNodePosition, BridgeCompWiring, BridgeGraphInput,
    BridgeInputKind, BridgeInputNode, BridgeReadNode,
};

/// A project with a node graph to wire, and a footage item to Read.
fn node_graph_to_wire() -> (ProjectReference, CompositionReference, Uuid) {
    let (project, _folder, filed, _loose) = project_with_folder();
    let graph = project
        .new_node_graph(String::new(), None)
        .expect("a node graph");
    (project, graph, filed.item_id())
}

/// The wiring as it stands, which is what every edit below starts from.
fn wiring_of(graph: &CompositionReference) -> BridgeCompWiring {
    graph.get_node_graph().expect("the graph").wiring
}

/// A box added, wired and placed is **one write and one undo step**, and the
/// node list comes back in the order it was sent (§3).
#[test]
fn a_read_is_added_wired_and_undone_in_one_step() {
    let (project, graph, item) = node_graph_to_wire();

    let mut wiring = wiring_of(&graph);
    let read = Uuid::now_v7();
    wiring.reads.push(BridgeReadNode {
        id: read,
        item,
        custom_name: None,
    });
    wiring.edges.push(BridgeCompEdge {
        from: read,
        from_port: "output".into(),
        to: wiring.output,
        to_port: "input".into(),
    });
    wiring.layout.push(BridgeCompNodePosition {
        node: read,
        x: -240.0,
        y: 20.0,
    });
    graph
        .set_node_graph(Vec::new(), wiring)
        .expect("a picture into the Output");

    let after = graph.get_node_graph().expect("the graph");
    assert_eq!(
        after
            .nodes
            .iter()
            .map(|n| (n.id, n.kind))
            .collect::<Vec<_>>(),
        vec![
            (read, BridgeCompNodeKind::Read),
            (after.wiring.output, BridgeCompNodeKind::Output),
        ],
        "the Reads come first and the Output last, as they were sent"
    );
    let brought_in = &after.nodes[0];
    assert_eq!(brought_in.label, "filed.mp4", "a Read draws under its item");
    assert!(!brought_in.missing);
    assert!(brought_in.item.is_some(), "and names it for the panel");
    assert!(
        brought_in.outputs.iter().all(|p| p.wired),
        "its picture socket has the wire on it"
    );
    assert!(after.nodes[1].inputs.iter().all(|p| p.wired));
    assert_eq!(after.wiring.layout.len(), 2);

    // One gesture, one undo step: the box, its wire and its place go together.
    project.undo().expect("undone");
    let back = graph.get_node_graph().expect("the graph");
    assert_eq!(back.nodes.len(), 1, "the undo took the box");
    assert!(back.wiring.edges.is_empty(), "and the wire with it");
}

/// Applying a graph to a layer binds the comp and **derives the rows from its
/// value Inputs** (§1.5), the first picture Input being the layer's own picture
/// and so no row at all.
#[test]
fn a_node_graph_effect_binds_its_comp_and_derives_its_rows() {
    let (project, graph, _) = node_graph_to_wire();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer(None).expect("a solid");

    let mut wiring = wiring_of(&graph);
    for (id, label, kind) in [
        ("source", "Source", BridgeInputKind::Picture),
        ("amount", "Amount", BridgeInputKind::Number),
    ] {
        wiring.inputs.push(BridgeInputNode {
            id: Uuid::now_v7(),
            input: BridgeGraphInput {
                id: id.into(),
                label: label.into(),
                kind,
                default: [0.5, 0.0, 0.0, 0.0],
                min: 0.0,
                max: 1.0,
                unit: BridgeUnit::Raw,
                preview: None,
            },
        });
    }
    graph
        .set_node_graph(Vec::new(), wiring)
        .expect("two Inputs");

    layer.add_node_graph_effect(&graph).expect("applied");
    let stack = layer.get_effects().expect("stack");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].name(), "node_graph");
    assert_eq!(
        stack[0].node_graph_comp_id(),
        Some(graph.id()),
        "the card's header names the graph it applies"
    );
    let rows: Vec<String> = stack[0]
        .get_info()
        .derived_params
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(
        rows,
        vec!["amount".to_owned()],
        "the value Inputs are rows; the first picture Input is the layer's own picture"
    );

    assert!(
        layer.add_node_graph_effect(&comp).is_err(),
        "a comp that is not a node graph has no graph to apply"
    );
}

// ---------------------------------------------------------------------------
// Round two: the placed graph, the box rows, the graph groups
// (docs/impl/node-graph-comp.md §5.3, §5.7, §5.8, §5.9, §5.11).

/// One Input declaration, so the tests below say only what they are about.
fn an_input(id: &str, kind: BridgeInputKind) -> BridgeInputNode {
    BridgeInputNode {
        id: Uuid::now_v7(),
        input: BridgeGraphInput {
            id: id.to_owned(),
            label: id.to_owned(),
            kind,
            default: [0.5, 0.0, 0.0, 0.0],
            min: 0.0,
            max: 1.0,
            unit: BridgeUnit::Raw,
            preview: None,
        },
    }
}

/// Give `graph` one value Input per name, keeping the ones it has.
fn a_graph_with_inputs(graph: &CompositionReference, ids: &[&str]) {
    let mut wiring = wiring_of(graph);
    for id in ids {
        wiring.inputs.push(an_input(id, BridgeInputKind::Number));
    }
    graph
        .set_node_graph(Vec::new(), wiring)
        .expect("the Inputs");
}

/// **Offered, never adopted** (§5.3): a layer that places a graph but carries
/// no Inputs, one from a file written before they existed or an import, is
/// handed a fresh instance, and the document only gains it when a row is
/// committed.
#[test]
fn a_layer_with_no_inputs_is_offered_a_fresh_one_and_adopts_it_on_commit() {
    let (project, graph, _) = node_graph_to_wire();
    let comp = add_comp(&project, "Scene");
    a_graph_with_inputs(&graph, &["amount"]);
    let layer = comp.add_precomp_layer(&graph, None).expect("placed");

    // Strip them, which is how a layer from an older file arrives.
    let state = project.state().expect("state");
    state
        .write()
        .expect("write")
        .store
        .commit(Op::SetLayerGraphInputs {
            comp: comp.id,
            layer: layer.layer_id,
            inputs: None,
        })
        .expect("stripped");
    assert!(layer.get_info().expect("info").graph_inputs.is_none());

    let revision = comp.document_revision().expect("a revision");
    let mut offered = layer
        .get_graph_inputs()
        .expect("asked")
        .expect("a placed graph is offered its Inputs");
    assert_eq!(offered.name(), "node_graph");
    assert_eq!(offered.node_graph_comp_id(), Some(graph.id()));
    assert_eq!(
        comp.document_revision().expect("a revision"),
        revision,
        "being offered them writes nothing"
    );

    offered
        .set_value(
            "amount".into(),
            BridgeEffectValue::Float(BridgeScalar::Static(0.75)),
        )
        .expect("the derived row takes a value");
    layer.set_effects(vec![offered], None).expect("committed");

    let landed = layer
        .get_info()
        .expect("info")
        .graph_inputs
        .expect("the Inputs are the document's now");
    assert!(
        landed.values.iter().any(|v| {
            v.id == "amount"
                && matches!(
                    v.value,
                    BridgeEffectValue::Float(BridgeScalar::Static(x))
                        if (x - 0.75).abs() < 1e-9
                )
        }),
        "the edited row landed, got {:?}",
        landed.values
    );
    assert_eq!(
        state
            .read()
            .expect("read")
            .store
            .history()
            .last()
            .expect("a step")
            .name,
        "Edit node graph inputs",
        "one step, named for what it did"
    );

    project.undo().expect("undone");
    assert!(
        layer.get_info().expect("info").graph_inputs.is_none(),
        "one undo step takes the whole adoption back"
    );
}

/// A graph group **saves the boxes and the wires between them**, and inserting
/// it mints fresh ids wired the same way, in one undo step (§5.8).
#[test]
fn a_graph_group_saves_its_boxes_and_inserts_them_fresh() {
    let (project, graph, _) = node_graph_to_wire();

    let blur = graph
        .new_graph_instance("blur".into(), None)
        .expect("a blur box");
    let glow = graph
        .new_graph_instance("glow".into(), None)
        .expect("a glow box");
    let (blur_id, glow_id) = (blur.id(), glow.id());
    let mut wiring = wiring_of(&graph);
    wiring.edges.push(BridgeCompEdge {
        from: blur_id,
        from_port: "output".into(),
        to: glow_id,
        to_port: "input".into(),
    });
    wiring.edges.push(BridgeCompEdge {
        from: glow_id,
        from_port: "output".into(),
        to: wiring.output,
        to_port: "input".into(),
    });
    wiring.layout.push(BridgeCompNodePosition {
        node: blur_id,
        x: -400.0,
        y: 0.0,
    });
    wiring.layout.push(BridgeCompNodePosition {
        node: glow_id,
        x: -200.0,
        y: 0.0,
    });
    graph
        .set_node_graph(vec![blur, glow], wiring)
        .expect("two boxes wired into the Output");

    let text = graph
        .save_graph_group("Soft".into(), 2, vec![blur_id, glow_id])
        .expect("saved");
    assert!(
        !text.contains("Output"),
        "the Output is never saved, or the insert would refuse itself"
    );

    let before = graph.get_node_graph().expect("the graph");
    graph
        .insert_graph_group(text.clone(), 40.0, 60.0)
        .expect("inserted");
    let after = graph.get_node_graph().expect("the graph");

    assert_eq!(after.nodes.len(), before.nodes.len() + 2, "two fresh boxes");
    let fresh: Vec<Uuid> = after
        .nodes
        .iter()
        .map(|n| n.id)
        .filter(|id| !before.nodes.iter().any(|n| n.id == *id))
        .collect();
    assert_eq!(fresh.len(), 2);
    assert!(
        after.wiring.edges.iter().any(|e| e.from == fresh[0]
            && e.to == fresh[1]
            && e.from_port == "output"
            && e.to_port == "input"),
        "the wire between them came with them, re-pointed at the fresh ids"
    );
    assert!(
        after
            .wiring
            .edges
            .iter()
            .any(|e| e.to == after.wiring.output),
        "and the graph's own Output wire is untouched"
    );
    assert_eq!(
        after.wiring.groups.last().expect("the group").name,
        "Soft",
        "the set lands named"
    );

    // One commit, so one undo step however many boxes it carried.
    project.undo().expect("undone");
    assert_eq!(
        graph.get_node_graph().expect("the graph").nodes.len(),
        before.nodes.len()
    );

    // Inserted twice, no two boxes share an id.
    graph
        .insert_graph_group(text.clone(), 0.0, 0.0)
        .expect("once");
    graph.insert_graph_group(text, 300.0, 0.0).expect("twice");
    let ids: Vec<Uuid> = graph
        .get_node_graph()
        .expect("the graph")
        .nodes
        .iter()
        .map(|n| n.id)
        .collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "every id is minted at insert");
}

/// **A motion path is the engine's own curve** (docs/07 §2.4): sampled once
/// per comp frame across the keyed range, landing exactly on its keys, with a
/// handle only where a side is eased. A still Position has no path at all.
#[test]
fn a_motion_path_is_sampled_by_the_engine_and_lands_on_its_keys() {
    use crate::api::effect::{
        BridgeBezierSide, BridgeKeyframe, BridgeRational, BridgeScalar, BridgeSideInterp,
    };
    use crate::api::layer::BridgeTransformProp;

    let (_project, layer) = project_with_layer();
    assert!(
        layer.motion_path().expect("answers").is_none(),
        "a still position draws no path"
    );

    let key = |frame: i64, value: f64, out: BridgeSideInterp| BridgeKeyframe {
        time: BridgeRational {
            num: frame,
            den: 30,
        },
        value,
        interp_in: BridgeSideInterp::Linear,
        interp_out: out,
    };
    let eased = BridgeSideInterp::Bezier(BridgeBezierSide {
        speed: 0.0,
        influence: 1.0 / 3.0,
    });
    layer
        .set_transforms(
            vec![
                BridgeTransformProp::PositionX,
                BridgeTransformProp::PositionY,
            ],
            vec![
                BridgeScalar::Keyframed(vec![
                    key(0, 100.0, eased),
                    key(30, 400.0, BridgeSideInterp::Linear),
                ]),
                BridgeScalar::Keyframed(vec![
                    key(0, 200.0, BridgeSideInterp::Linear),
                    key(30, 500.0, BridgeSideInterp::Linear),
                ]),
            ],
        )
        .expect("keyed");

    let path = layer
        .motion_path()
        .expect("answers")
        .expect("a keyed position has a path");
    assert_eq!(path.first_frame, 0);
    assert_eq!(
        path.samples.len(),
        31 * 2,
        "one sample per frame from the first key to the last"
    );

    // The samples are lumit-core's own evaluation at each frame.
    let stored = layer.item().expect("the layer");
    for (i, xy) in path.samples.chunks(2).enumerate() {
        let t = i as f64 / 30.0;
        assert!(
            (xy[0] - stored.transform.position_x.value_at(t)).abs() < 1e-9,
            "x at frame {i}"
        );
        assert!(
            (xy[1] - stored.transform.position_y.value_at(t)).abs() < 1e-9,
            "y at frame {i}"
        );
    }
    // x leaves its first key eased, so a third of the way along it is nowhere
    // near the straight line's third.
    assert!(
        (path.samples[20] - 200.0).abs() > 1.0,
        "the eased x is not a straight line"
    );

    assert_eq!(path.keys.len(), 2);
    let (first, last) = (&path.keys[0], &path.keys[1]);
    assert_eq!((first.frame, first.x, first.y), (0, 100.0, 200.0));
    assert_eq!((last.frame, last.x, last.y), (30, 400.0, 500.0));
    assert_eq!((first.x_index, first.y_index), (Some(0), Some(0)));
    assert!(
        first.handle_in.is_none(),
        "an end key has no span to lean into"
    );
    // x is eased flat, y is straight: the handle sits on the key's own x and
    // a third of the way along y's chord.
    let out = first.handle_out.expect("an eased side draws a handle");
    assert!(
        (out[0] - 100.0).abs() < 1e-9 && (out[1] - 300.0).abs() < 1e-9,
        "handle {out:?}"
    );
    assert!(
        last.handle_in.is_none(),
        "straight on both axes draws no handle"
    );
}

// ---------------------------------------------------------------------------
// The Addons page's four engine answers (docs/impl/addons.md §5).
// ---------------------------------------------------------------------------

/// One test at a time wherever the addons folder is pointed at one of its own.
///
/// `with_dir` is process-wide, because an install runs on a worker thread and
/// a thread-local would not follow it there. The suite runs in parallel, so
/// two tests overlapping would read each other's addons.
fn addons_serially() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(|held| held.into_inner())
}

/// A manifest for a pack whose one download is a plain file of `bytes` bytes.
/// Written here rather than fetched: the page's whole contract with the engine
/// is this text plus a list of paths.
fn an_addon_manifest(id: &str, format: u32, bytes: u64) -> String {
    format!(
        r#"{{"format":{format},"id":"{id}","kind":"model","name":"Test pack",
           "version":"1.0","summary":"A depth map from a single frame",
           "licence":"Apache-2.0","licence_url":"https://example.invalid/licence",
           "size":{bytes},"requires":["runtime"],
           "platforms":{{"any":{{"downloads":[
             {{"url":"https://example.invalid/model.onnx",
               "sha256":"0000000000000000000000000000000000000000000000000000000000000001",
               "size":{bytes},"unpack":"file","dest":"model.onnx"}}]}}}},
           "model":{{"task":"depth","arch":"depth-anything","file":"model.onnx"}}}}"#
    )
}

/// The page installs what Dart fetched, reads the row back, removes it, and
/// meets each of its refusals as a typed variant rather than a sentence it
/// would have to parse.
///
/// The addons folder is pointed at a temporary one for the length of the test,
/// because the install runs wherever frb puts it and must never write into the
/// user's own. The one refusal not driven here is `AddonBusy`: the install slot
/// is private to `lumit-ml`, and the rule is pinned there, in
/// `a_second_install_is_refused_rather_than_queued`.
#[test]
fn an_addon_installs_lists_and_removes_and_is_refused_honestly() {
    use crate::api::addons::{
        addon_install, addon_list, addon_remove, addon_runtime, addons_dir, BridgeAddonKind,
        BridgeRuntimeState,
    };

    let _serial = addons_serially();
    let root = tempfile::tempdir().expect("a folder");
    let downloads = tempfile::tempdir().expect("a folder");
    lumit_ml::store::with_dir(Some(root.path().to_path_buf()));

    assert_eq!(
        addons_dir().as_deref(),
        Some(root.path().to_string_lossy().as_ref()),
        "Dart and Rust read one folder, not two"
    );
    assert!(addon_list().is_empty());

    let fetched = downloads.path().join("model.onnx");
    std::fs::write(&fetched, vec![0u8; 12]).expect("the download");
    let path = fetched.to_string_lossy().into_owned();

    addon_install(an_addon_manifest("depth-pack", 1, 12), vec![path.clone()]).expect("installed");

    let listed = addon_list();
    assert_eq!(listed.len(), 1);
    let row = &listed[0];
    assert_eq!(row.id, "depth-pack");
    assert_eq!(row.kind, BridgeAddonKind::Model);
    assert_eq!(row.name, "Test pack");
    assert_eq!(row.task, "depth", "the word the row shows");
    assert_eq!(row.licence, "Apache-2.0");
    assert_eq!(row.size_bytes, 12);
    assert!(!row.broken);

    // With no runtime installed, the row says so, and every model button
    // reads "Needs the runtime" off that.
    assert_eq!(addon_runtime().state, BridgeRuntimeState::Missing);

    // A manifest from a newer build is refused with the engine's own sentence,
    // which is the only thing the page can say about it.
    let refusal = addon_install(an_addon_manifest("newer", 2, 12), vec![path])
        .expect_err("a newer format is refused");
    let BridgeError::AddonInvalid(why) = &refusal else {
        panic!("the wrong refusal: {refusal:?}");
    };
    assert!(why.contains("newer build"), "{why}");

    // An id nothing is installed under is missing, not invalid: the page has
    // a different sentence for each.
    assert!(matches!(
        addon_remove("nothing-here".to_owned()),
        Err(BridgeError::AddonMissing)
    ));

    addon_remove("depth-pack".to_owned()).expect("removed");
    assert!(addon_list().is_empty());

    lumit_ml::store::with_dir(None);
}

// A Photoshop document comes in as a comp of its layers and a folder of layer
// items, in one undo step, and anything else is left to the footage import.
#[cfg(feature = "media")]
#[test]
fn a_photoshop_document_imports_as_a_comp_of_its_layers_in_one_undo_step() {
    use lumit_core::model::{BlendMode, EffectValue, LayerKind};
    use lumit_media::psd::fixture::{described, document, Layer, Value};

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("poster.psd");
    let mut hat = Layer::solid("Hat", [8, 8, 16, 16], [0, 0, 255, 255]);
    hat.opacity = 128;
    hat.blend = *b"mul ";
    let mut props = Layer::group("Props", true);
    props.visible = false;
    // A drop shadow lit from 120 degrees, which is Photoshop's own default.
    let mut title = Layer::solid("Title", [4, 4, 12, 28], [255, 255, 255, 255]);
    let black = ["Rd  ", "Grn ", "Bl  "].map(|key| (key, Value::Number(0.0)));
    let shadow = Value::Object(vec![
        ("enab", Value::Switch(true)),
        ("Clr ", Value::Object(black.to_vec())),
        ("Opct", Value::Number(40.0)),
        ("lagl", Value::Number(120.0)),
    ]);
    title.blocks = vec![described(*b"lfx2", &[("DrSh", shadow)])];
    // Threshold at 128 of 255, with a mask over the left half.
    let mut threshold = Layer::solid("Threshold", [0, 0, 0, 0], [0; 4]);
    threshold.blocks = vec![(*b"thrs", vec![0, 128, 0, 0])];
    threshold.mask = Some(([0, 0, 32, 16], 0, vec![255; 32 * 16]));
    // Bottom first, as the file lists them. "Empty" has no picture, "Sky" is
    // a fill layer, and "Brim" is a group inside "Props".
    let layers = [
        Layer::solid("Background", [0, 0, 32, 32], [255, 0, 0, 255]),
        Layer::group("</Layer group>", false),
        hat,
        Layer::group("</Layer group>", false),
        Layer::solid("Feather", [0, 0, 4, 4], [255, 255, 0, 255]),
        Layer::group("Brim", true),
        Layer::solid("Scarf", [0, 0, 8, 8], [0, 255, 0, 255]),
        props,
        Layer::solid("Empty", [0, 0, 0, 0], [0; 4]),
        Layer::fill("Sky", [0.0, 0.0, 255.0]),
        threshold,
        title,
    ];
    std::fs::write(&path, document(32, 32, 8, &layers)).expect("the fixture writes");

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let snapshot = || {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot()
    };
    let before = snapshot().items.len();

    let left_out = project
        .import_layers(path.to_string_lossy().into_owned())
        .expect("the document imports");
    assert_eq!(left_out, Some(1), "the layer with no picture is counted");

    let doc = snapshot();
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.name == "poster" => Some(c),
            _ => None,
        })
        .expect("a comp named for the file");
    assert_eq!((comp.width, comp.height), (32, 32));
    let names: Vec<&str> = comp.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Title",
            "Threshold",
            "Threshold",
            "Sky",
            "Scarf",
            "Brim",
            "Hat",
            "Background"
        ],
        "top first"
    );

    // Each layer reads its own record of the file, by its place in the list.
    let picks: Vec<Option<u32>> = comp
        .layers
        .iter()
        .map(|l| match &l.kind {
            LayerKind::Footage { item } => match doc.item(*item) {
                Some(ProjectItem::Footage(f)) => f.source_layer,
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        picks,
        [
            Some(11),
            None,
            Some(10),
            None,
            Some(6),
            None,
            Some(2),
            Some(0)
        ]
    );

    // An adjustment layer is an Adjustment layer carrying the nearest effect.
    // Its mask is the record read as a picture, on a hidden layer under it
    // that it takes as its matte.
    let (threshold, mask) = (&comp.layers[1], &comp.layers[2]);
    assert_eq!(threshold.kind, LayerKind::Adjustment);
    assert!(!threshold.switches.visible, "it arrives switched off");
    assert_eq!(threshold.matte.map(|m| m.layer), Some(mask.id));
    assert!(!mask.switches.visible);
    let [effect] = threshold.effects.as_slice() else {
        panic!("the adjustment carries one effect");
    };
    assert_eq!(effect.effect.match_name, "threshold");
    let level = effect.params.iter().find(|p| p.id == "level");
    let Some(EffectValue::Float(level)) = level.map(|p| &p.value) else {
        panic!("Threshold has a level");
    };
    assert!((level.value_at(0.0) - 50.2).abs() < 0.1, "128 of 255");

    let hat = &comp.layers[6];
    assert_eq!(hat.blend, BlendMode::Multiply);
    let opacity = hat.transform.opacity.value_at(0.0);
    assert!((opacity - 50.2).abs() < 0.1, "128 of 255 is {opacity}");
    assert!(!hat.switches.visible, "a hidden group hides its members");
    assert!(comp.layers[0].switches.visible);

    assert_eq!(comp.groups.len(), 1);
    assert_eq!(comp.groups[0].name, "Props");
    let members: Vec<_> = comp.layers[4..7].iter().map(|l| l.id).collect();
    assert_eq!(comp.groups[0].members, members);

    // A group inside a group is a composition of its own, placed in the outer
    // group as a Precomp layer that lets its layers blend through.
    let brim = &comp.layers[5];
    let LayerKind::Precomp { comp: inner } = brim.kind else {
        panic!("the inner group is a Precomp layer");
    };
    assert!(brim.switches.collapse);
    let inner = doc.comp(inner).expect("the inner group's composition");
    let names: Vec<&str> = inner.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Feather"]);

    // A fill layer is a Solid layer of its colour.
    let LayerKind::Solid { def } = comp.layers[3].kind else {
        panic!("the fill is a Solid layer");
    };
    let Some(ProjectItem::Solid(sky)) = doc.item(def) else {
        panic!("the fill has a solid");
    };
    assert_eq!(sky.colour.0, [0.0, 0.0, 1.0, 1.0]);

    // A layer style is the Lumit style of the same name. Light from 120
    // degrees throws a shadow at 150.
    let [shadow] = comp.layers[0].styles.as_slice() else {
        panic!("the title wears one style");
    };
    assert_eq!(shadow.effect.match_name, "style_drop_shadow");
    let row = |id: &str| match shadow.params.iter().find(|p| p.id == id) {
        Some(param) => match &param.value {
            EffectValue::Float(value) => value.value_at(0.0),
            _ => f64::NAN,
        },
        None => f64::NAN,
    };
    assert_eq!((row("opacity"), row("direction")), (40.0, 150.0));

    let folder = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Folder(f) if f.name == "poster layers" => Some(f),
            _ => None,
        })
        .expect("a folder for the layer items");
    // Five pictures, the adjustment's mask, the fill's solid and the inner
    // group's composition.
    assert_eq!(folder.children.len(), 8);

    project.undo().expect("one import, one step");
    assert_eq!(snapshot().items.len(), before, "undo takes all of it back");

    // A document with one layer is a still, and a still is not a document.
    let flat = dir.path().join("flat.psd");
    let one = [Layer::solid("Background", [0, 0, 32, 32], [255, 0, 0, 255])];
    std::fs::write(&flat, document(32, 32, 8, &one)).expect("the fixture writes");
    for plain in [flat, dir.path().join("photo.png")] {
        let answer = project.import_layers(plain.to_string_lossy().into_owned());
        assert_eq!(answer.expect("no error"), None);
    }
    assert_eq!(snapshot().items.len(), before, "and nothing was added");

    // The adjustment kinds whose numbers are reshaped on the way in. Over two
    // pictures, bottom first: Photoshop's own Warming Filter (85), a green
    // curve turned upside down, a tinted Black and White, a Vibrance with a
    // Saturation, a negative Vibrance, which is left out, and a Colour
    // Balance.
    let adjust = |name: &str, block: ([u8; 4], Vec<u8>)| {
        let mut layer = Layer::solid(name, [0, 0, 0, 0], [0; 4]);
        layer.blocks = vec![block];
        layer
    };
    let words = |lead: &[u8], words: &[i16], tail: &[u8]| {
        let words = words.iter().flat_map(|w| w.to_be_bytes());
        [lead, &words.collect::<Vec<u8>>(), tail].concat()
    };
    let white = ["Rd  ", "Grn ", "Bl  "].map(|key| (key, Value::Number(255.0)));
    let grey = [
        ("Rd  ", Value::Number(-10.0)),
        ("Yllw", Value::Number(60.0)),
        ("Grn ", Value::Number(40.0)),
        ("Cyn ", Value::Number(60.0)),
        ("Bl  ", Value::Number(20.0)),
        ("Mgnt", Value::Number(80.0)),
        ("useTint", Value::Switch(true)),
        ("tintColor", Value::Object(white.to_vec())),
    ];
    let vibrance = |vibrance: f64| {
        let items = [("vibrance", vibrance), ("Strt", -20.0)].map(|(k, v)| (k, Value::Number(v)));
        described(*b"vibA", &items)
    };
    let curve = words(&[0], &[1, 0, 4, 2, 255, 0, 0, 255], &[0]);
    let balance = words(&[], &[0, 0, 0, 100, 0, -100, 0, 0, 100], &[1, 0, 0, 0]);
    // The older layout, in Lab: 67.06, 32 and 120, then 25 per cent.
    let glass = words(&[], &[2, 7, 6706, 3200, 12000, 0, 0, 25], &[1, 0, 0, 0]);
    let layers = [
        Layer::solid("Background", [0, 0, 32, 32], [255, 0, 0, 255]),
        Layer::solid("Hat", [8, 8, 16, 16], [0, 0, 255, 255]),
        adjust("Glass", (*b"phfl", glass)),
        adjust("Curves", (*b"curv", curve)),
        adjust("Grey", described(*b"blwh", &grey)),
        adjust("Vibrance", vibrance(30.0)),
        adjust("Faded", vibrance(-30.0)),
        adjust("Balance", (*b"blnc", balance)),
    ];
    let grade = dir.path().join("grade.psd");
    std::fs::write(&grade, document(32, 32, 8, &layers)).expect("the fixture writes");
    let left_out = project.import_layers(grade.to_string_lossy().into_owned());
    assert_eq!(left_out.expect("the document imports"), Some(1));
    let doc = snapshot();
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.name == "grade" => Some(c),
            _ => None,
        })
        .expect("a comp named for the file");
    let carried: Vec<Vec<&str>> = comp
        .layers
        .iter()
        .map(|l| l.effects.iter().map(|e| &*e.effect.match_name).collect())
        .collect();
    assert_eq!(
        carried,
        [
            vec!["colour_balance"],
            vec!["vibrancy", "saturation"],
            vec!["black_and_white"],
            vec!["curves"],
            vec!["photo_filter"],
            vec![],
            vec![]
        ]
    );
    let row = |layer: usize, effect: usize, id: &str| -> Vec<f64> {
        let params = &comp.layers[layer].effects[effect].params;
        match params.iter().find(|p| p.id == id).map(|p| &p.value) {
            Some(EffectValue::Float(v)) => vec![v.value_at(0.0)],
            Some(EffectValue::Colour(c)) => c.iter().map(|v| v.value_at(0.0)).collect(),
            Some(EffectValue::Bool(on)) => vec![f64::from(u8::from(*on))],
            Some(EffectValue::Curve(points)) => {
                points.iter().flatten().map(|v| f64::from(*v)).collect()
            }
            _ => Vec::new(),
        }
    };
    assert_eq!(
        row(3, 0, "green"),
        [0.0, 1.0, 1.0, 0.0],
        "input, then output"
    );
    assert_eq!(row(3, 0, "master"), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(
        (row(2, 0, "reds"), row(2, 0, "tint")),
        (vec![-10.0], vec![1.0])
    );
    assert_eq!(row(2, 0, "tint_colour"), [1.0; 4]);
    assert_eq!(
        (row(1, 0, "amount"), row(1, 1, "saturation")),
        (vec![30.0], vec![80.0])
    );
    assert_eq!(row(0, 0, "lift"), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        row(0, 0, "gamma"),
        [2.0, 1.0, 0.5, 1.0],
        "a stop a full slider"
    );
    assert_eq!(row(0, 0, "gain"), [1.0, 1.0, 2.0, 1.0]);
    // That filter is the orange Photoshop shows as 236, 138, 0.
    let glass = row(4, 0, "colour");
    assert!((glass[0] - 0.84).abs() < 0.01 && (glass[1] - 0.255).abs() < 0.01);
    assert_eq!((glass[2], row(4, 0, "density")), (0.0, vec![25.0]));
    assert_eq!(row(4, 0, "preserve_luminosity"), [1.0]);
}

// An Illustrator document comes in the same way: a comp the artboard's size
// with a layer for each top-level layer, top first, and a hidden layer hidden.
// One with a single layer is footage, and is still read by its own reader.
#[cfg(feature = "media")]
#[test]
fn an_illustrator_document_imports_as_a_comp_of_its_layers() {
    use lumit_core::model::LayerKind;
    use lumit_media::ai::fixture::{document, Layer};

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("logo.ai");
    let mut guide = Layer::solid("Guide", [0, 0, 4, 4], [0, 255, 0]);
    guide.visible = false;
    let layers = [
        Layer::solid("Background", [0, 0, 32, 48], [255, 0, 0]),
        Layer::solid("Mark", [8, 8, 16, 16], [0, 0, 255]),
        guide,
    ];
    std::fs::write(&path, document(48, 32, &layers)).expect("the fixture writes");

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let snapshot = || {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot()
    };
    let before = snapshot().items.len();

    let left_out = project
        .import_layers(path.to_string_lossy().into_owned())
        .expect("the document imports");
    assert_eq!(left_out, Some(0));

    let doc = snapshot();
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.name == "logo" => Some(c),
            _ => None,
        })
        .expect("a comp named for the file");
    assert_eq!((comp.width, comp.height), (48, 32));
    let layers: Vec<(&str, bool, Option<u32>)> = comp
        .layers
        .iter()
        .map(|l| {
            let pick = match &l.kind {
                LayerKind::Footage { item } => match doc.item(*item) {
                    Some(ProjectItem::Footage(f)) => f.source_layer,
                    _ => None,
                },
                _ => None,
            };
            (l.name.as_str(), l.switches.visible, pick)
        })
        .collect();
    assert_eq!(
        layers,
        [
            ("Guide", false, Some(2)),
            ("Mark", true, Some(1)),
            ("Background", true, Some(0))
        ],
        "top first, each reading its own layer of the file"
    );
    assert!(doc.items.iter().any(|i| match i {
        ProjectItem::Folder(f) => f.name == "logo layers" && f.children.len() == 3,
        _ => false,
    }));

    project.undo().expect("one import, one step");
    assert_eq!(snapshot().items.len(), before, "undo takes all of it back");

    // One layer is a picture, not a document. ffmpeg can't open it, so the
    // footage item it becomes has to probe through the file's own reader.
    let flat = dir.path().join("flat.ai");
    let one = [Layer::solid("Layer 1", [0, 0, 32, 48], [255, 0, 0])];
    std::fs::write(&flat, document(48, 32, &one)).expect("the fixture writes");
    let answer = project.import_layers(flat.to_string_lossy().into_owned());
    assert_eq!(answer.expect("no error"), None);
    let probe = crate::probe::ensure_probed(flat.as_path()).expect("the document probes");
    let video = probe.video.as_ref().expect("a picture");
    assert_eq!((video.width, video.height), (48, 32));
}

/// A transform row's expression keeps its language, and the value it had
/// underneath, through the op that writes it. Undo puts back exactly what was
/// there and redo writes the same note again, seed and all.
#[test]
fn a_transform_expression_keeps_its_language_through_undo() {
    use crate::api::effect::{
        sample_scalar_with_context, BridgeExpressionLanguage, BridgeRational, BridgeScalar,
    };
    use crate::api::layer::BridgeTransformProp;

    let (project, layer) = project_with_layer();
    let rotation = BridgeTransformProp::Rotation;
    let read = || layer.get_transform().expect("transform").rotation;
    let shown = || sample_scalar_with_context(read(), BridgeRational { num: 0, den: 1 }, layer);
    layer
        .set_transform(rotation, BridgeScalar::Static(30.0))
        .expect("a number");

    let typed = BridgeScalar::Expression(
        "value + Math.round(7 / 2)".into(),
        BridgeExpressionLanguage::JavaScript,
    );
    layer
        .set_transform(rotation, typed.clone())
        .expect("an expression");
    assert_eq!(read(), typed);
    assert_eq!(shown(), 34.0, "30 underneath, and 3.5 rounded up");

    // The same text as Rhai is a different expression, and says so.
    layer
        .set_transforms(
            vec![rotation],
            vec![BridgeScalar::Expression(
                "7 / 2".into(),
                BridgeExpressionLanguage::Rhai,
            )],
        )
        .expect("the same row in Rhai");
    assert_eq!(shown(), 3.0);
    project.undo().expect("undone");
    assert_eq!(read(), typed);
    assert_eq!(shown(), 34.0);

    project.undo().expect("undone");
    assert_eq!(read(), BridgeScalar::Static(30.0));
    project.redo().expect("redone");
    assert_eq!(read(), typed);
    assert_eq!(shown(), 34.0);
}
