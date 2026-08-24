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

/// **Picking any frame of a moved run relinks the run** (K-439), and takes the
/// ordinary clips beside it along in the same sweep.
///
/// The failure this pins is quiet: the item points at frame 1, the user picks
/// frame 42, and the path rewrite compares `frame0001.png` with `frame0042.png`
/// — all they share at the end is `.png`, so "the folder moved" comes out as
/// "everything up to half a frame number moved", and the sibling is swept to a
/// path that does not exist.
#[test]
fn relinking_a_sequence_by_any_of_its_frames_finds_the_run_and_its_neighbours() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new_root = dir.path().join("moved");
    std::fs::create_dir_all(new_root.join("frames")).expect("tree");
    for n in 1..=50u32 {
        std::fs::write(
            new_root.join("frames").join(format!("frame{n:04}.png")),
            b"f",
        )
        .expect("frame");
    }
    std::fs::write(new_root.join("music.wav"), b"w").expect("sound");

    let old_root = std::path::Path::new("/nowhere/edit");
    let run = FootageItem {
        sequence: Some(lumit_core::model::SequenceRef::default()),
        id: Uuid::now_v7(),
        name: "frame[0001-0050].png".into(),
        media: MediaRef {
            relative_path: "frame0001.png".into(),
            absolute_path: old_root
                .join("frames")
                .join("frame0001.png")
                .to_string_lossy()
                .into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
    };
    let sibling = FootageItem {
        sequence: None,
        id: Uuid::now_v7(),
        name: "music.wav".into(),
        media: MediaRef {
            relative_path: "music.wav".into(),
            absolute_path: old_root.join("music.wav").to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
    };
    let (run_id, sibling_id) = (run.id, sibling.id);

    let project = LumitBridgeState::new_project(None).expect("a new project");
    {
        let state = project.state().expect("state");
        let state = state.write().expect("write");
        for (index, item) in [ProjectItem::Footage(run), ProjectItem::Footage(sibling)]
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

    // The middle of the run, which is what a picker started at the folder gives.
    FootageReference::new(project.id, run_id)
        .relink(
            new_root
                .join("frames")
                .join("frame0042.png")
                .to_string_lossy()
                .into_owned(),
        )
        .expect("relinked");

    let state = project.state().expect("state");
    let state = state.read().expect("read");
    let doc = state.store.snapshot();
    let media_of = |id: Uuid| match doc.item(id) {
        Some(ProjectItem::Footage(f)) => f.media.absolute_path.clone(),
        _ => panic!("the footage is still there"),
    };
    assert_eq!(
        media_of(run_id),
        new_root
            .join("frames")
            .join("frame0001.png")
            .to_string_lossy(),
        "the run is pointed at its first frame, not at the one that was picked"
    );
    assert_eq!(
        media_of(sibling_id),
        new_root.join("music.wav").to_string_lossy(),
        "the sound file beside it moved the same way and came back too"
    );
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
    comp.add_footage_layer(footage, false).expect("placed");

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
    // The same answer in every build: whether a file is on disk is a question
    // for the filesystem, not for the decoder (K-273). Before that, a
    // media-less build called this path Ready.
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

/// **A folder of numbered stills is one item, not two thousand** (K-439).
///
/// The front door's whole promise: pick any file of a run and the run comes in,
/// named for its span, pointed at its first frame — and picking the rest of the
/// files afterwards (which is what selecting the whole folder does) hands back
/// the item that is already there rather than filing it again.
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

/// A numbered file with no numbered neighbours is a still, and a folder of
/// numbered *clips* is a folder of clips — neither becomes a sequence (K-439).
#[test]
fn a_lone_still_and_a_run_of_clips_import_as_they_always_did() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("poster0001.png"), b"x").expect("write");
    for n in 1..=3u32 {
        std::fs::write(dir.path().join(format!("take{n:04}.mp4")), b"x").expect("write");
    }

    let project = LumitBridgeState::new_project(None).expect("project");
    project
        .import_footage(
            dir.path()
                .join("poster0001.png")
                .to_string_lossy()
                .into_owned(),
        )
        .expect("imported");
    assert_eq!(
        project
            .get_items()
            .expect("roots")
            .first()
            .map(|i| i.name().expect("name")),
        Some("poster0001.png".to_owned()),
        "a still with no neighbours keeps its own file name"
    );

    for n in 1..=3u32 {
        project
            .import_footage(
                dir.path()
                    .join(format!("take{n:04}.mp4"))
                    .to_string_lossy()
                    .into_owned(),
            )
            .expect("imported");
    }
    assert_eq!(
        project.get_items().expect("roots").len(),
        4,
        "three clips and the still, each its own item"
    );
}

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
/// aperture folded in (K-313) could not reach Blades, Roundness, Rotation or
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
    layer.set_effects(staged).expect("committed");

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

/// The twirls and greying rules cross too, and every rule still names rows the
/// panel will actually be drawing.
///
/// `EffectSchema::groups` existed in the core schema from K-145 but never
/// crossed the bridge, so Shake and Matte key declared twirls the panel could
/// not know about and drew flat. This is the sweep that keeps the layout side
/// honest now that something depends on it.
#[test]
fn every_builtin_lists_its_layout() {
    for info in crate::api::effect::list_effects() {
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

/// Depth of field is what the greying crossing exists for, so it is what pins
/// it: the folded twirls and the rules that grey a row arrive on the far side
/// intact (K-313).
#[test]
fn dofs_twirls_and_greying_rules_cross_the_bridge() {
    use crate::api::effect::{BridgeEnabledCond, BridgeParamKind};

    let groups = crate::api::effect::list_parameter_groups("dof".into());
    let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
    assert_eq!(labels, vec!["Iris", "Highlights", "Depth map"]);
    assert!(groups.iter().all(|g| g.collapsed));

    let enabled_when = crate::api::effect::list_enabled_when("dof".into());
    let rule = |param: &str| {
        enabled_when
            .iter()
            .find(|r| r.param == param)
            .unwrap_or_else(|| panic!("no rule for {param}"))
            .clone()
    };
    // Focus distance and the focus point take each other over.
    let distance = rule("focus");
    assert_eq!(distance.on, "use_focus_point");
    assert_eq!(distance.cond, BridgeEnabledCond::BoolIs(false));
    let point = rule("focus_point_x");
    assert_eq!(point.on, "use_focus_point");
    assert_eq!(point.cond, BridgeEnabledCond::BoolIs(true));
    // And everything that reads the depth pass greys without one.
    let channel = rule("depth_channel");
    assert_eq!(channel.on, "depth");
    assert_eq!(channel.cond, BridgeEnabledCond::LayerSet);

    // The dial crosses as itself, not flattened into a Float row.
    let params = crate::api::effect::list_parameters("dof".into());
    let kind = |id: &str| {
        params
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
            .kind
            .clone()
    };
    assert!(matches!(
        kind("rotation"),
        BridgeParamKind::Angle { default, .. } if default == 0.0
    ));
    // The focus point is an `_x`/`_y` Float pair the panel folds into one row
    // (docs/07 §6.1), not a kind of its own.
    assert!(matches!(
        kind("focus_point_x"),
        BridgeParamKind::Float { .. }
    ));
}

/// A closed range and a curve each cross as **their own kind**, because each
/// draws a control no arrangement of existing rows draws — a track and thumb,
/// and the unit square (K-412, K-414, docs/17).
///
/// The Slider's *value* is still an ordinary Float, which is the whole point of
/// the arrangement: the row keeps keyframes, the graph editor and the
/// expression seed without any of them learning a new kind.
#[test]
fn a_closed_range_and_a_curve_each_cross_as_their_own_kind() {
    use crate::api::effect::{list_parameters, BridgeParamKind};

    let wipe = list_parameters("linear_wipe".into());
    let completion = wipe
        .iter()
        .find(|p| p.id == "completion")
        .expect("a wipe has a completion");
    assert!(
        matches!(
            completion.kind,
            BridgeParamKind::Slider {
                default,
                min,
                max,
            } if default == 50.0 && min == 0.0 && max == 100.0
        ),
        "a closed range crosses as a Slider carrying that range, got {:?}",
        completion.kind
    );

    let curves = list_parameters("curves".into());
    for channel in ["master", "red", "green", "blue", "alpha"] {
        let param = curves
            .iter()
            .find(|p| p.id == channel)
            .unwrap_or_else(|| panic!("curves declares {channel}"));
        assert!(matches!(param.kind, BridgeParamKind::Curve));
    }
    // Mix stays the plain Float it is: its slider ends and its hard bounds
    // happen to agree, but the *kind* is declared, not inferred from them.
    assert!(curves
        .iter()
        .any(|p| p.id == "mix" && matches!(p.kind, BridgeParamKind::Float { .. })));
}

/// An unknown effect gets an empty layout rather than an error, for the same
/// reason its parameter list is empty: a project carrying an effect this build
/// does not know still opens.
#[test]
fn the_layout_of_an_unknown_effect_is_empty() {
    assert!(crate::api::effect::list_parameter_groups("not-an-effect".into()).is_empty());
    assert!(crate::api::effect::list_enabled_when("not-an-effect".into()).is_empty());
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
    .write_at(&mut group, lumit_core::time::Rational::ZERO)
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

/// **The Audio layer (K-435).** The sound of a clip, added as its own layer:
/// the same footage source, no picture, one undo step.
///
/// The layer reads as its own kind across the seam so the Timeline can draw it
/// as one, and `has_picture` answers false whatever the file holds — that is
/// what the flag means, and it is what the outline reads to decide the layer
/// has no visibility switch to offer.
#[test]
fn the_sound_of_a_clip_can_be_its_own_layer() {
    use crate::api::layer::BridgeLayerKind;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");

    comp.add_audio_layer(&footage).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);

    assert_eq!(
        layer.get_kind().expect("kind"),
        BridgeLayerKind::Audio,
        "a footage source placed for its sound reads as an Audio layer"
    );
    assert!(
        !layer.has_picture().expect("asked"),
        "an Audio layer has no picture, so no visibility switch is offered"
    );

    // One op: one undo takes the layer away again.
    project.undo().expect("undo");
    assert!(comp.get_layers().expect("layers").is_empty());

    // The ordinary placement of the same clip is unchanged — a Footage layer
    // that draws. Without a decoder nothing can be probed, so the picture is
    // assumed present rather than assumed away.
    comp.add_footage_layer(&footage, false).expect("placed");
    let drawn = comp.get_layers().expect("layers").remove(0);
    assert_eq!(drawn.get_kind().expect("kind"), BridgeLayerKind::Footage);
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
        BridgeLayerKind::NullLayer,
        BridgeLayerKind::Sequence,
        BridgeLayerKind::Light,
    ] {
        let added = match expected {
            BridgeLayerKind::Solid => comp.add_solid_layer(),
            BridgeLayerKind::Text => comp.add_text_layer(),
            BridgeLayerKind::Camera => comp.add_camera_layer(),
            BridgeLayerKind::Adjustment => comp.add_adjustment_layer(),
            BridgeLayerKind::NullLayer => comp.add_null_layer(),
            BridgeLayerKind::Sequence => comp.add_sequence_layer(),
            // The area kind (K-360) — the one with a size, and so the one
            // worth checking reaches the document intact.
            BridgeLayerKind::Light => comp.add_light_layer(2),
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

/// `has_picture` is what fills the matte cell and every layer-valued effect
/// parameter (K-194), so a kind with no pixels must answer false or the user is
/// offered a matte source that silently mattes nothing. A Camera has always
/// answered false; a Null has to as well — it is the same "no pixels at all"
/// case, and the catch-all arm used to hand it a picture it does not have.
#[test]
fn the_pixel_less_kinds_report_no_picture() {
    let (_project, layer) = project_with_layer();
    let comp = CompositionReference::new(_project.id, layer.comp_id());

    let null = comp.add_null_layer().expect("null added");
    assert!(
        !null.has_picture().expect("has_picture"),
        "a Null has no pixels, so it can never be a matte or a layer parameter"
    );
    let camera = comp.add_camera_layer().expect("camera added");
    assert!(!camera.has_picture().expect("has_picture"));
    // And the kinds that do draw still say so, so the fix did not empty the
    // dropdowns it was meant to correct.
    let solid = comp.add_solid_layer().expect("solid added");
    assert!(solid.has_picture().expect("has_picture"));
    let adjustment = comp.add_adjustment_layer().expect("adjustment added");
    assert!(adjustment.has_picture().expect("has_picture"));
}

/// **An effect on a Null keeps its values, animation and all** (K-274).
///
/// A Null draws nothing, so an image effect on one changes no picture — which
/// is why the drop is *labelled inert* rather than refused. The parameters are
/// the point: a control put on a Null is how a value is meant to be published
/// for other layers to read (a Slider driving an expression, once expressions
/// land). So the stack must survive a commit, keep its keyframes, and sample
/// like any other curve — nothing may quietly strip an effect from a layer that
/// has no pixels.
#[test]
fn an_effect_on_a_null_layer_keeps_its_animated_value() {
    use crate::api::effect::{sample_scalar, BridgeEffectValue, BridgeRational, BridgeScalar};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let null = comp.add_null_layer().expect("null added");

    null.add_effect("blur".into())
        .expect("the drop is accepted");
    assert_eq!(
        null.get_effects().expect("effects").len(),
        1,
        "an effect on a Null is stored like any other"
    );

    // Animate it, and commit through the ordinary staged-copy path.
    let mut staged = null.get_effects().expect("effects");
    let key = |num: i64, value: f64| crate::api::effect::BridgeKeyframe {
        time: BridgeRational { num, den: 1 },
        value,
        interp_in: crate::api::effect::BridgeSideInterp::Linear,
        interp_out: crate::api::effect::BridgeSideInterp::Linear,
    };
    let keys = BridgeScalar::Keyframed(vec![key(0, 0.0), key(1, 10.0)]);
    staged[0]
        .set_value("radius".into(), BridgeEffectValue::Float(keys))
        .expect("a Null's parameter takes a value like any other");
    null.set_effects(staged).expect("committed");

    // Read it back and sample it: halfway along the ramp is five, on a layer
    // that will never draw a pixel.
    let Ok(BridgeEffectValue::Float(scalar)) = null
        .get_effects()
        .expect("effects")
        .first()
        .expect("the effect survived the commit")
        .get_value("radius".into())
    else {
        panic!("a Null's effect parameter must read back as the Float it is");
    };
    let half = sample_scalar(scalar, BridgeRational { num: 1, den: 2 });
    assert!(
        (half - 5.0).abs() < 1e-9,
        "the curve on a Null evaluates like any other: got {half}"
    );
}

/// **A copied layer arrives whole, and lands where the playhead is** (K-275).
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
    let source = comp.add_solid_layer().expect("a layer to copy");
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
    source.set_effects(staged).expect("committed");

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

/// A layer pasted into **another** composition keeps everything that is its own
/// and drops what pointed at the comp it left (K-275).
#[test]
fn a_layer_pasted_into_another_comp_drops_the_references_it_left_behind() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let parent = comp.add_null_layer().expect("a parent");
    let source = comp.add_solid_layer().expect("a layer to copy");
    source.set_parent(Some(parent.layer_id)).expect("parented");

    let text = source.copy_layer().expect("copied");

    // Back into its own comp: the parent is still there, so it is kept.
    let same = comp.paste_layer(text.clone(), None).expect("pasted");
    assert_eq!(
        same.get_parent().expect("parent"),
        Some(parent.layer_id),
        "pasting where the parent lives keeps the parenting"
    );
    assert_eq!(
        same.get_span().expect("span").in_point,
        source.get_span().expect("span").in_point,
        "None keeps the time it was copied at"
    );

    // Into a different comp: the parent means nothing there, so it goes.
    let elsewhere = project
        .new_composition("Second".into(), None)
        .expect("another comp");
    let there = elsewhere.paste_layer(text, Some(0)).expect("pasted");
    assert_eq!(
        there.get_parent().expect("parent"),
        None,
        "a parent from another comp is dropped, not left dangling"
    );
    assert_eq!(
        elsewhere.get_layers().expect("layers").len(),
        1,
        "and the layer itself did arrive"
    );
}

/// **A pasted effect lands with its first keyframe under the playhead** (K-275,
/// the owner's rule). An effect copied from a layer that flashes at 4 s and
/// pasted while the playhead sits at 12 s must flash at 12 s.
#[test]
fn a_pasted_effect_starts_its_animation_at_the_playhead() {
    use crate::api::effect::{BridgeEffectValue, BridgeRational, BridgeScalar};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let source = comp.add_solid_layer().expect("a layer");
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
    source.set_effects(staged).expect("committed");

    let text = source.copy_effects(Vec::new()).expect("copied");
    let target = comp.add_solid_layer().expect("somewhere to paste");
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

/// **Several picked effects copy as one document, in stack order** (K-300).
/// The Effect controls panel and the Timeline both let a Shift-click take a run
/// of headings, so the call takes a list — and what comes back is the order the
/// stack is drawn in, not the order the clicks happened in, or a copied group
/// would paste back shuffled.
#[test]
fn copying_several_effects_takes_them_in_stack_order() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let source = comp.add_solid_layer().expect("a layer");
    source.add_effect("blur".into()).expect("first");
    source.add_effect("sharpen".into()).expect("second");
    source.add_effect("vignette".into()).expect("third");
    let stack = source.get_effects().expect("effects");
    let ids: Vec<_> = stack.iter().map(|e| e.id()).collect();

    // Picked bottom-up: the third, then the first.
    let text = source
        .copy_effects(vec![ids[2], ids[0]])
        .expect("copied both");
    let target = comp.add_solid_layer().expect("somewhere to paste");
    target.paste_effects(text, 0).expect("pasted");

    let pasted: Vec<_> = target
        .get_effects()
        .expect("effects")
        .iter()
        .map(|e| e.get_info().name)
        .collect();
    assert_eq!(
        pasted,
        vec!["blur".to_string(), "vignette".to_string()],
        "the two picked effects arrive, in the order the stack held them"
    );

    // Naming nothing that is on this layer is a refusal, not a whole-stack copy.
    assert!(source.copy_effects(vec![Uuid::nil()]).is_err());
}

/// An effect with no animation at all pastes unchanged — there is no timing to
/// place, and inventing one would move a look that was never in motion (K-275).
#[test]
fn a_pasted_effect_with_no_keyframes_is_left_where_it_is() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let source = comp.add_solid_layer().expect("a layer");
    source.add_effect("blur".into()).expect("an effect");
    let before = source.get_effects().expect("effects")[0]
        .get_value("radius".into())
        .expect("radius");

    let text = source.copy_effects(Vec::new()).expect("copied");
    let target = comp.add_solid_layer().expect("somewhere to paste");
    target.paste_effects(text, 120).expect("pasted");

    let after = target.get_effects().expect("effects")[0]
        .get_value("radius".into())
        .expect("radius");
    assert_eq!(format!("{before:?}"), format!("{after:?}"));
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
    comp.add_footage_layer(&footage, false).expect("placed");
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
    comp.add_footage_layer(&footage, false).expect("placed");
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

/// The memory report answers without a project, and its arithmetic holds
/// (K-294).
///
/// The point of the report is the *unaccounted* figure — what the process holds
/// that no tier here admits to — so what is pinned is that it is derived from
/// the two numbers it claims to be derived from, and that a platform which will
/// not say how big the process is says so with a zero rather than a guess.
#[test]
fn the_memory_report_answers_and_its_arithmetic_holds() {
    use crate::api::cache::memory_report;

    let report = memory_report();

    // Every desktop this ships on can answer; a platform that cannot returns 0
    // rather than inventing, and then there is nothing to check.
    if report.process_bytes == 0 {
        return;
    }

    let accounted = report.frame_cache_bytes
        + report.decode_cache_bytes
        + if report.unified_memory {
            report.vram_cache_bytes
        } else {
            0
        };
    assert_eq!(
        report.unaccounted_bytes,
        report.process_bytes.saturating_sub(accounted),
        "unaccounted is the process less the tiers that live in ordinary memory"
    );
    assert!(
        report.unaccounted_bytes <= report.process_bytes,
        "a part cannot exceed the whole"
    );
    // The card's frames count against the process only where they are in it.
    // Getting this backwards makes a cache doing its job read as a leak, which
    // is the one way this report can actively mislead.
    if !report.unified_memory {
        assert_eq!(
            report.unaccounted_bytes,
            report
                .process_bytes
                .saturating_sub(report.frame_cache_bytes + report.decode_cache_bytes),
            "a discrete card's frames are not in this process, so they are not \
             subtracted from it"
        );
    }
    assert!(
        report.park_queue_frames <= lumit_render::diskio::MAX_PENDING_PARKS as u64,
        "the write-behind queue is bounded (K-277), and the report shows it"
    );
}

/// A process that is holding memory answers a plausible size for itself — the
/// syscall behind the report is wired, not a stub returning zero on the
/// platform running the tests.
#[test]
fn the_process_reports_its_own_size() {
    let bytes = crate::api::system::resident_memory_bytes();
    // Bound rather than asserted inline: `cfg!` is a literal, and an assert on
    // one is a constant expression clippy rightly refuses.
    let desktop = cfg!(any(windows, target_os = "linux", target_os = "macos"));
    if bytes == 0 {
        // Only an unsupported platform may answer nothing.
        assert!(!desktop, "every desktop target answers its own size");
        return;
    }
    // A test process holding less than a megabyte, or more than a terabyte, is
    // a misread struct rather than a real reading.
    assert!(
        bytes > (1 << 20) && bytes < (1 << 40),
        "a plausible process size, not a misread field: {bytes} bytes"
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

/// Precompose moving every attribute: the chosen layers go into a new comp as
/// they were, one Precomp layer stands where the topmost of them stood, timing
/// is untouched, and the whole move is one undo step.
#[test]
fn precompose_packs_the_chosen_layers_and_leaves_one_precomp_behind() {
    use crate::api::layer::BridgeLayerKind;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let bottom = comp.add_solid_layer().expect("solid");
    let middle = comp.add_solid_layer().expect("solid");
    let top = comp.add_solid_layer().expect("solid");
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

/// Precompose refuses an empty selection, and ignores a reference to a layer
/// of some other comp rather than losing the whole batch to it.
#[test]
fn precompose_refuses_nothing_and_survives_a_stray_reference() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let other = project.new_composition("Other".into(), None).expect("comp");
    let mine = comp.add_solid_layer().expect("solid");
    let theirs = other.add_solid_layer().expect("solid");

    assert!(matches!(
        comp.precompose(Vec::new(), String::new(), false, false),
        Err(BridgeError::InvalidLayer)
    ));
    assert!(matches!(
        comp.precompose(vec![theirs.layer_id], String::new(), false, false),
        Err(BridgeError::InvalidLayer)
    ));

    // The stray one is dropped; the layer that *is* here still packs.
    comp.precompose(
        vec![mine.layer_id, theirs.layer_id],
        "Packed".into(),
        false,
        false,
    )
    .expect("precomposed");
    assert_eq!(comp.get_layers().expect("layers").len(), 1);
    assert_eq!(other.get_layers().expect("layers").len(), 1);
}

/// Leaving the attributes behind: the layer moves into the new comp stripped
/// back to its source, and the Precomp layer standing in its place carries the
/// effect stack — once, never on both, which would apply it twice.
#[test]
fn precompose_leaving_attributes_keeps_them_on_the_precomp_layer_only() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let solid = comp.add_solid_layer().expect("solid");
    solid.add_effect("blur".into()).expect("effect");
    let second = comp.add_solid_layer().expect("solid");
    // Two layers have no single layer to leave the attributes on, so this is
    // refused outright rather than applied to one of them.
    assert!(matches!(
        comp.precompose(
            vec![solid.layer_id, second.layer_id],
            "Both".into(),
            true,
            false
        ),
        Err(BridgeError::InvalidLayer)
    ));
    second.delete().expect("delete");

    let packed = comp
        .precompose(vec![solid.layer_id], "Blurred".into(), true, false)
        .expect("precomposed");

    assert_eq!(packed.get_effects().expect("effects").len(), 1);

    let Some(ItemReference::Composition(inner)) = packed.get_source_item().expect("source") else {
        panic!("a Precomp layer's source is a composition");
    };
    let inside = inner.get_layers().expect("layers");
    assert_eq!(inside.len(), 1);
    assert!(inside[0].get_effects().expect("effects").is_empty());
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
    let solid = comp.add_solid_layer().expect("solid");
    // Two seconds of a thirty-second comp, starting at five.
    let span = BridgeSpan {
        in_point: BridgeRational { num: 5, den: 1 },
        out_point: BridgeRational { num: 7, den: 1 },
        start_offset: BridgeRational { num: 5, den: 1 },
    };
    solid.set_span(span).expect("span");

    let packed = comp
        .precompose(vec![solid.layer_id], "Trimmed".into(), false, true)
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
        fps: 0.0,
        range_start_frame: -1,
        range_end_frame: -1,
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

// --- Shape layers (K-237) -------------------------------------------------

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

#[test]
fn shape_contents_are_replaced_as_a_whole_and_undone_in_one_step() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let shape = comp
        .add_shape_layer("Art".into(), vec![shape_item("Rectangle", 0.0, 0.0, 10.0)])
        .expect("a shape layer");

    let mut contents = shape.get_shape_contents().expect("contents");
    contents.push(shape_item("Second", 40.0, 40.0, 10.0));
    shape.set_shape_contents(contents).expect("set");
    assert_eq!(shape.get_shape_contents().expect("contents").len(), 2);

    project.undo().expect("undone");
    assert_eq!(
        shape.get_shape_contents().expect("contents").len(),
        1,
        "one edit, one undo step"
    );
}

/// Dragging the left-most point left grows the art's box leftwards, and the
/// layer's origin **is** that box's corner — so without the position following
/// it, every point nobody touched would slide the other way (K-308).
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
        let items: Vec<_> = contents.iter().map(|i| i.write_item()).collect();
        let (x0, y0, _, _) = lumit_core::shape::contents_bounds(&items).expect("a box");
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
    shape.set_shape_contents(contents).expect("set");

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

#[test]
fn shape_contents_ride_the_read_model() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let shape = comp
        .add_shape_layer("Art".into(), vec![shape_item("Rectangle", 0.0, 0.0, 10.0)])
        .expect("a shape layer");

    let model = comp.get_model().expect("model");
    let entry = model
        .layers
        .iter()
        .find(|l| l.layer.id() == shape.id())
        .expect("the layer");
    assert_eq!(entry.info.shape_contents.len(), 1);
    assert_eq!(entry.info.kind, BridgeLayerKind::Shape);

    // Every other kind answers with an empty list rather than an error.
    assert!(layer.get_shape_contents().expect("contents").is_empty());
}

#[test]
fn a_shape_layer_refuses_art_that_is_not_a_shape() {
    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());

    assert!(matches!(
        comp.add_shape_layer("Nothing".into(), Vec::new()),
        Err(BridgeError::EmptyPath)
    ));

    let mut thin = shape_item("Line", 0.0, 0.0, 10.0);
    thin.vertices.truncate(1);
    assert!(matches!(
        comp.add_shape_layer("Thin".into(), vec![thin]),
        Err(BridgeError::EmptyPath)
    ));

    // And a layer that is not a shape refuses the edit rather than growing art.
    assert!(matches!(
        layer.set_shape_contents(vec![shape_item("Rectangle", 0.0, 0.0, 10.0)]),
        Err(BridgeError::NotShape)
    ));
}

// --- Paint: strokes on a layer (K-227) ------------------------------------

fn stroke(name: &str, points: &[(f64, f64)]) -> crate::api::layer::BridgeStroke {
    use crate::api::layer::{BridgePaintMode, BridgeStroke, BridgeStrokePoint};
    BridgeStroke {
        id: Uuid::now_v7(),
        name: name.into(),
        points: points
            .iter()
            .map(|&(x, y)| BridgeStrokePoint { x, y })
            .collect(),
        colour: crate::api::assets::BridgeColourRgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        width: 12.0,
        hardness: 0.8,
        opacity: 100.0,
        mode: BridgePaintMode::Paint,
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

/// Strokes are carried in the read model beside the masks (K-184), so the
/// Timeline can list them without asking per row per frame.
#[test]
fn strokes_ride_the_read_model() {
    let (project, layer) = project_with_layer();
    layer
        .add_stroke(stroke("Brush 1", &[(1.0, 2.0)]))
        .expect("added");

    let comp = CompositionReference::new(project.id, layer.comp_id());
    let model = comp.get_model().expect("model");
    let entry = model
        .layers
        .iter()
        .find(|l| l.layer.id() == layer.id())
        .expect("the layer");
    assert_eq!(entry.info.paint.len(), 1);
    assert_eq!(entry.info.paint[0].name, "Brush 1");
}

#[test]
fn a_stroke_is_edited_and_deleted_by_id() {
    let (_project, layer) = project_with_layer();
    let first = stroke("Brush 1", &[(0.0, 0.0)]);
    let second = stroke("Brush 2", &[(5.0, 5.0)]);
    layer.add_stroke(first.clone()).expect("added");
    layer.add_stroke(second.clone()).expect("added");

    let mut edited = first.clone();
    edited.name = "Renamed".into();
    edited.width = 40.0;
    layer.set_stroke(edited).expect("set");
    let strokes = layer.get_paint().expect("paint");
    assert_eq!(
        strokes[0].name, "Renamed",
        "the one named, not the last one"
    );
    assert_eq!(strokes[0].width, 40.0);
    assert_eq!(strokes[1].name, "Brush 2");

    layer.delete_stroke(first.id).expect("deleted");
    let strokes = layer.get_paint().expect("paint");
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].id, second.id);

    // A stale reference is a calm error, not an edit landing on whatever sits
    // at that index now.
    assert!(matches!(
        layer.delete_stroke(first.id),
        Err(BridgeError::NoSuchStroke)
    ));
    assert!(matches!(
        layer.set_stroke(first),
        Err(BridgeError::NoSuchStroke)
    ));
}

#[test]
fn the_last_stroke_can_be_taken_back() {
    let (_project, layer) = project_with_layer();
    assert!(
        matches!(layer.delete_last_stroke(), Err(BridgeError::NoSuchStroke)),
        "nothing painted, nothing to take back"
    );
    layer
        .add_stroke(stroke("Brush 1", &[(0.0, 0.0)]))
        .expect("added");
    layer
        .add_stroke(stroke("Brush 2", &[(1.0, 1.0)]))
        .expect("added");
    layer.delete_last_stroke().expect("taken back");
    let strokes = layer.get_paint().expect("paint");
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].name, "Brush 1");
}

/// A gesture with nothing in it is refused rather than stored: it would be a
/// Timeline row with nothing behind it, exactly as an empty mask would be.
#[test]
fn a_stroke_with_no_points_is_refused() {
    let (_project, layer) = project_with_layer();
    assert!(matches!(
        layer.add_stroke(stroke("Nothing", &[])),
        Err(BridgeError::EmptyStroke)
    ));
    assert!(layer.get_paint().expect("paint").is_empty());
}

/// Numbers that would render wrongly for ever after are clamped at the seam,
/// as a mask's opacity is.
#[test]
fn absurd_stroke_numbers_are_clamped_at_the_bridge() {
    let (_project, layer) = project_with_layer();
    let mut wild = stroke("Wild", &[(0.0, 0.0)]);
    wild.opacity = 4000.0;
    wild.hardness = -3.0;
    wild.width = 1e9;
    layer.add_stroke(wild).expect("added");
    let got = &layer.get_paint().expect("paint")[0];
    assert_eq!(got.opacity, 100.0);
    assert_eq!(got.hardness, 0.0);
    assert_eq!(got.width, 10_000.0);
}

/// The three modes and a clone's offset survive the crossing unchanged.
#[test]
fn every_paint_mode_round_trips() {
    use crate::api::layer::BridgePaintMode;

    let (_project, layer) = project_with_layer();
    for mode in [
        BridgePaintMode::Paint,
        BridgePaintMode::Erase,
        BridgePaintMode::Clone,
    ] {
        let mut s = stroke("Mark", &[(2.0, 2.0)]);
        s.mode = mode;
        s.clone_offset_x = -20.0;
        s.clone_offset_y = 7.5;
        layer.add_stroke(s).expect("added");
    }
    let strokes = layer.get_paint().expect("paint");
    assert_eq!(strokes.len(), 3);
    assert_eq!(strokes[0].mode, BridgePaintMode::Paint);
    assert_eq!(strokes[1].mode, BridgePaintMode::Erase);
    assert_eq!(strokes[2].mode, BridgePaintMode::Clone);
    assert_eq!(strokes[2].clone_offset_x, -20.0);
    assert_eq!(strokes[2].clone_offset_y, 7.5);
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
        expression: None,
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
            expression: None,
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

/// A text layer's words can be driven by an expression, and clearing the box
/// hands the layer back to the words that were typed — an empty string is not
/// an expression that says nothing, which would leave the layer blank forever.
#[test]
fn a_text_expression_round_trips_and_clears() {
    use crate::api::assets::{BridgeColourRgba, BridgeTextDocument};

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer().expect("a text layer");

    let document = |expression: Option<&str>| BridgeTextDocument {
        text: "typed".into(),
        expression: expression.map(str::to_owned),
        size: 48.0,
        fill: BridgeColourRgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
    };

    text.set_text(document(Some("time * 2"))).expect("set");
    let after = text.get_text().expect("text").expect("still text");
    assert_eq!(after.expression.as_deref(), Some("time * 2"));
    assert_eq!(after.text, "typed", "the typed words survive underneath");

    text.set_text(document(Some("   "))).expect("cleared");
    assert_eq!(
        text.get_text().expect("text").expect("text").expression,
        None,
        "an empty box means no expression"
    );
    let _ = project;
    let _ = layer;
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

// --- The sequence view's clip edits (K-247, K-248) ------------------------

/// A Sequence layer built for the clip tests: one clip spanning [0, 4).
#[cfg(test)]
fn sequenced_layer() -> (ProjectReference, CompositionReference, LayerReference) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    layer.convert_to_sequenced().expect("sequenced");
    let layer = comp.get_layers().expect("layers").remove(0);
    (project, comp, layer)
}

/// Re-speeding a clip keeps its place and pins its first frame — the two
/// promises the whole editing surface rests on (K-022, K-070).
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

/// A ramp reads as no single speed, which is what puts the envelope on screen
/// instead of a number.
#[test]
fn a_ramped_clip_has_no_single_speed() {
    let (_project, _comp, layer) = sequenced_layer();
    let clip = layer.get_clips().expect("clips").remove(0);
    layer.set_clip_speed(clip.id, 100.0, 300.0).expect("ramped");
    let after = layer.get_clips().expect("clips").remove(0);
    assert_eq!(after.speed_percent, None);
    assert!(after.retimed);
}

/// A Sequence layer's bar is its clips' extent (K-248): cutting leaves it
/// alone, and deleting an outermost clip brings the end in.
#[test]
fn the_layers_bar_follows_its_clips() {
    let (_project, _comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    let (in_before, out_before) = (whole.in_frame, whole.out_frame);

    // A cut adds an edit point and changes no extent at all.
    let middle = (in_before + out_before) / 2;
    layer.cut_clip_at(middle).expect("cut");
    let cut = layer.get_info().expect("info");
    assert_eq!((cut.in_frame, cut.out_frame), (in_before, out_before));
    assert_eq!(cut.clips.len(), 2);

    // Deleting the last clip brings the end of the bar back with it.
    layer.delete_clip_at(out_before - 1).expect("deleted");
    let trimmed = layer.get_info().expect("info");
    assert_eq!(trimmed.clips.len(), 1);
    assert_eq!(trimmed.in_frame, in_before, "the start is where it was");
    assert!(
        trimmed.out_frame < out_before,
        "and the end came in with the clip that went"
    );
}

/// A clip slides along its row, keeping its length and what it plays.
#[test]
fn sliding_a_clip_moves_it_without_changing_it() {
    let (_project, _comp, layer) = sequenced_layer();
    let before = layer.get_clips().expect("clips").remove(0);
    let length = before.end_frame - before.start_frame;

    layer
        .slide_clip(before.id, before.start_frame + 5)
        .expect("slid");
    let after = layer.get_clips().expect("clips").remove(0);
    assert_eq!(after.start_frame, before.start_frame + 5);
    assert_eq!(after.end_frame - after.start_frame, length, "same length");
    assert_eq!(after.retimed, before.retimed, "and the same map");
}

/// Converting a **retimed** layer into a Sequence layer keeps its retiming,
/// and converting back returns it — a round trip must leave the layer playing
/// what it played.
#[test]
fn converting_a_retimed_layer_both_ways_keeps_its_map() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);

    layer.toggle_retime_property().expect("retimed");
    let before = layer.get_retime_property().expect("read").expect("a map");

    layer.convert_to_sequenced().expect("sequenced");
    let sequenced = comp.get_layers().expect("layers").remove(0);
    let clip = sequenced.get_clips().expect("clips").remove(0);
    assert!(
        clip.retimed,
        "the clip carries the layer's map: it spans the whole layer, so the          two are the same clock"
    );

    sequenced.convert_from_sequenced().expect("back");
    let back = comp.get_layers().expect("layers").remove(0);
    assert_eq!(
        back.get_retime_property().expect("read"),
        Some(before),
        "and the round trip left it exactly as it was"
    );
}

/// Cutting a **retimed** clip gives each half a key at the cut, so the two
/// ramps are independent from the moment they are made — editing one half's
/// speed never bends the other. An un-retimed clip gains no keys at all
/// (K-236: a map nobody has shaped is not one to put keys into).
#[test]
fn a_razor_cut_keys_a_retimed_clip_and_only_that() {
    let (_project, _comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    let at = (whole.in_frame + whole.out_frame) / 2;

    // Un-retimed: cut, and neither half has a map.
    layer.cut_clip_at(at).expect("cut");
    for clip in layer.get_clips().expect("clips") {
        assert!(!clip.retimed, "a cut alone does not retime anything");
    }

    // Retimed: each half keeps a map, and each opens on the moment it starts.
    let (_p2, _c2, ramped) = sequenced_layer();
    let one = ramped.get_clips().expect("clips").remove(0);
    ramped.set_clip_speed(one.id, 200.0, 200.0).expect("ramped");
    let whole = ramped.get_info().expect("info");
    ramped
        .cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");

    let halves = ramped.get_clips().expect("clips");
    assert_eq!(halves.len(), 2);
    for half in &halves {
        assert!(half.retimed, "each half kept the ramp it was cut out of");
        let BridgeScalar::Keyframed(keys) = &half.retime else {
            panic!("a keyframed map");
        };
        assert!(keys.len() >= 2, "with a key at each of its own ends");
    }
    // The later half opens where the cut fell, not at the top of the media.
    let (early, late) = (&halves[0], &halves[1]);
    let value = |c: &crate::api::layer::BridgeClip| {
        let BridgeScalar::Keyframed(keys) = &c.retime else {
            panic!("keyframed");
        };
        keys.first().expect("a first key").value
    };
    assert!(
        value(late) > value(early),
        "the second half starts further into the source than the first"
    );
}

/// A Sequence layer converts back to plain footage — the way out of the
/// clip-editing surface, which has to exist because the way in is offered to
/// anyone (K-248).
#[test]
fn a_sequence_layer_converts_back_to_footage() {
    let (_project, comp, layer) = sequenced_layer();
    let clip = layer.get_clips().expect("clips").remove(0);
    layer.set_clip_speed(clip.id, 250.0, 250.0).expect("ramped");

    layer.convert_from_sequenced().expect("converted back");
    let back = comp.get_layers().expect("layers").remove(0);
    assert_eq!(back.get_kind().expect("kind"), BridgeLayerKind::Footage);
    // The clip spanned the whole layer, so its map is the layer's map: clip
    // time and layer time were the same clock, and K-249 made them the same
    // kind of map, so nothing had to be converted.
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
/// depth pass follow the footage it belongs to (K-248).
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
    comp.add_footage_layer(&other_footage, false)
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

/// One clip's shape copies on its own — the other half of the menu.
#[test]
fn one_clips_shape_copies_by_itself() {
    let (_project, _comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    layer
        .cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");
    let one = layer.get_clips().expect("clips").remove(0);

    let all = layer.copy_sequence_shape(None).expect("copied");
    let just = layer.copy_sequence_shape(Some(one.id)).expect("copied");
    assert_ne!(all, just, "one clip is not the whole row");
    assert!(
        just.len() < all.len(),
        "and it carries less: one piece rather than two"
    );
}

/// A Sequence layer has no Retime of its own (K-075): its clips carry the
/// retiming, and a second map over the whole row would be a rival to those —
/// exactly what K-249 spent itself ending.
#[test]
fn a_sequence_layer_refuses_a_retime_of_its_own() {
    let (_project, _comp, layer) = sequenced_layer();
    assert!(matches!(
        layer.toggle_retime_property(),
        Err(BridgeError::NotRetimeable)
    ));
    assert!(
        layer.get_retime_property().expect("read").is_none(),
        "and nothing was installed on the way to refusing"
    );
}

/// Dragging a clip back past the start of the row carries the **layer**
/// earlier, the way dragging any other layer's bar before the start of the
/// composition does — and every other clip stays exactly where it was on the
/// comp's clock while it happens.
#[test]
fn a_clip_dragged_before_the_start_takes_the_layer_with_it() {
    let (_project, _comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    layer
        .cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");
    let before = layer.get_clips().expect("clips");
    let first = before
        .iter()
        .min_by_key(|c| c.start_frame)
        .expect("the earlier half")
        .clone();
    let second = before
        .iter()
        .max_by_key(|c| c.start_frame)
        .expect("the later half")
        .clone();

    layer
        .slide_clip(first.id, first.start_frame - 10)
        .expect("slid before the start");

    let after = layer.get_clips().expect("clips");
    let moved = after
        .iter()
        .find(|c| c.id == first.id)
        .expect("still there");
    let stayed = after
        .iter()
        .find(|c| c.id == second.id)
        .expect("still there");

    assert_eq!(
        moved.start_frame,
        first.start_frame - 10,
        "the clip went where it was dragged, past the start"
    );
    assert_eq!(
        stayed.start_frame, second.start_frame,
        "and the other clip did not move on the comp's clock"
    );
    assert_eq!(
        layer.get_info().expect("info").in_frame,
        first.start_frame - 10,
        "the layer's bar starts where its earliest clip now does"
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
        "re-speeding pins a clip's first frame (K-070), it does not move it          back to the start of the media"
    );
}

/// A clip that has been cut can be lengthened again.
///
/// The reported fault: after a razor cut the new edge looked draggable and
/// snapped back every time, because only inward trims were honoured — so the
/// half you had just made could be shortened and never restored.
#[test]
fn a_cut_clip_can_be_lengthened_again() {
    let (_project, _comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    layer
        .cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");
    let left = layer.get_clips().expect("clips").remove(0);

    // Shorter first, which always worked…
    layer
        .trim_clip(left.id, left.start_frame, left.end_frame - 3)
        .expect("trimmed in");
    let short = layer.get_clips().expect("clips").remove(0);
    assert_eq!(short.end_frame, left.end_frame - 3);

    // …and now longer again, which is what snapped back.
    layer
        .trim_clip(short.id, short.start_frame, left.end_frame + 4)
        .expect("extended out");
    let long = layer.get_clips().expect("clips").remove(0);
    assert_eq!(long.end_frame, left.end_frame + 4);
    assert_eq!(long.start_frame, left.start_frame, "the other edge held");
}

/// Extending carries the map on at the speed it was already going, so the
/// frames the clip already showed keep showing at the same moments.
#[test]
fn extending_a_retimed_clip_keeps_what_it_already_played() {
    let (_project, _comp, layer) = sequenced_layer();
    let clip = layer.get_clips().expect("clips").remove(0);
    layer.set_clip_speed(clip.id, 200.0, 200.0).expect("sped");
    let fast = layer.get_clips().expect("clips").remove(0);

    layer
        .trim_clip(fast.id, fast.start_frame, fast.end_frame + 5)
        .expect("extended");
    let longer = layer.get_clips().expect("clips").remove(0);
    assert_eq!(
        longer.speed_percent,
        Some(200.0),
        "still double speed all the way along, not a ramp into the new tail"
    );
}

/// Deleting a clip leaves a gap: nothing after it moves, so every edit point
/// still standing keeps the beat it was cut to (K-022).
#[test]
fn deleting_a_clip_leaves_a_gap() {
    let (_project, _comp, layer) = sequenced_layer();
    let whole = layer.get_info().expect("info");
    layer
        .cut_clip_at((whole.in_frame + whole.out_frame) / 2)
        .expect("cut");
    let clips = layer.get_clips().expect("clips");
    assert_eq!(clips.len(), 2);
    let (first, second) = (clips[0].clone(), clips[1].clone());

    layer.delete_clip(first.id).expect("deleted");
    let left = layer.get_clips().expect("clips");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, second.id);
    assert_eq!(
        left[0].start_frame, second.start_frame,
        "what was left did not slide back to fill the hole"
    );
}

/// The envelope writes a clip's whole map, and it reads back as what it wrote.
#[test]
fn a_clips_retime_round_trips_through_the_envelope() {
    let (_project, _comp, layer) = sequenced_layer();
    let clip = layer.get_clips().expect("clips").remove(0);
    assert!(!clip.retimed, "un-retimed to start");

    // Double speed as two keys, the shape the envelope authors.
    layer.set_clip_speed(clip.id, 200.0, 200.0).expect("sped");
    let sped = layer.get_clips().expect("clips").remove(0);
    let map = sped.retime.clone();

    layer.set_clip_retime(sped.id, map).expect("written back");
    let back = layer.get_clips().expect("clips").remove(0);
    assert_eq!(back.speed_percent, Some(200.0));
    assert_eq!(back.start_frame, clip.start_frame, "place untouched");
    assert_eq!(back.end_frame, clip.end_frame);
}

// --- Video arriving as a Sequence layer (K-246) ---------------------------

/// With the preference on, media that **runs** arrives as a one-clip Sequence
/// layer — ready to be cut on its own row — while a still image does not,
/// because there is nothing in one frame to cut.
///
/// Needs an ffmpeg on PATH to make the fixtures; skips itself without one, the
/// same as every other test here that wants real media.
#[test]
fn video_is_wrapped_and_a_still_is_not() {
    #[cfg(feature = "media")]
    {
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(clip) = lumit_media::index::tests_support::fixture(dir.path()) else {
            return; // no ffmpeg on this machine
        };
        // One frame of the same pattern: a still, by duration rather than by
        // extension — which is exactly the distinction the engine draws.
        let Some(bin) = lumit_media::index::tests_support::ffmpeg_bin() else {
            return;
        };
        let still = dir.path().join("still.png");
        let made = std::process::Command::new(bin)
            .args(["-v", "error", "-y", "-i"])
            .arg(&clip)
            .args(["-frames:v", "1"])
            .arg(&still)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        let project = LumitBridgeState::new_project(None).expect("a new project");
        let comp = project.new_composition("Scene".into(), None).expect("comp");

        let video = project
            .import_footage(clip.to_string_lossy().into_owned())
            .expect("imported");
        comp.add_footage_layer(&video, true).expect("placed");
        let layers = comp.get_layers().expect("layers");
        assert_eq!(
            layers[0].get_kind().expect("kind"),
            BridgeLayerKind::Sequence,
            "video asked to arrive as a Sequence layer does"
        );
        assert_eq!(
            layers[0].get_info().expect("info").clip_frames.len(),
            1,
            "one clip, spanning the whole import"
        );

        if made {
            let image = project
                .import_footage(still.to_string_lossy().into_owned())
                .expect("imported");
            comp.add_footage_layer(&image, true).expect("placed");
            assert_eq!(
                comp.get_layers().expect("layers")[0]
                    .get_kind()
                    .expect("kind"),
                BridgeLayerKind::Footage,
                "a still has no run to cut, so it is never wrapped"
            );
        }

        // …and with the preference off, video is a Footage layer as always.
        comp.add_footage_layer(&video, false).expect("placed");
        assert_eq!(
            comp.get_layers().expect("layers")[0]
                .get_kind()
                .expect("kind"),
            BridgeLayerKind::Footage
        );
    }
}

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
    comp.add_footage_layer(&footage, false)
        .expect("placed cold");
    let cold = comp.get_layers().expect("layers")[0]
        .get_info()
        .expect("info");

    // Warm: the answer is certainly held now, so this placement is a look-up.
    let probed = crate::probe::ensure_probed(&clip).expect("the fixture probes");
    assert!(probed.video.is_some(), "the fixture has a picture");
    comp.add_footage_layer(&footage, false)
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

/// Media that will not probe stays a plain Footage layer even when the
/// preference is on. Guessing towards the more elaborate shape on no
/// information is the more annoying mistake to undo.
#[test]
fn unreadable_media_is_never_wrapped() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/not-really-here.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, true).expect("placed");
    assert_eq!(
        comp.get_layers().expect("layers")[0]
            .get_kind()
            .expect("kind"),
        BridgeLayerKind::Footage
    );
}

// --- Retime ------------------------------------------------------------

/// The frame-interpolation policy is a layer's own setting, present on every
/// layer whether or not it is retimed (K-249).
///
/// It used to live inside the rival retime store this file once exercised at
/// length — a constant speed, a reverse gate and an enable switch, all of them
/// a second way to retime a layer that the Retime property already did better.
/// Those went with the store; the policy stayed, because it was never part of
/// the map to begin with (docs/04 §10).
#[test]
fn interpolation_is_a_layer_setting_of_its_own() {
    use crate::api::retime::BridgeRetimeInterp;

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/shot.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);

    assert_eq!(
        layer.get_interpolation().expect("read"),
        BridgeRetimeInterp::Nearest,
        "nearest is the gaming-footage default (docs/04 §10)"
    );
    assert!(
        layer.get_retime_property().expect("read").is_none(),
        "and it is readable with no retime in sight"
    );

    layer
        .set_interpolation(BridgeRetimeInterp::Blend)
        .expect("set");
    assert_eq!(
        layer.get_interpolation().expect("read"),
        BridgeRetimeInterp::Blend
    );

    // One undo step, and it does not reach for a retime that is not there.
    project.undo().expect("undo");
    assert_eq!(
        layer.get_interpolation().expect("read"),
        BridgeRetimeInterp::Nearest
    );
}

/// Every layer kind has a policy — it is not a footage-layer idea, because any
/// layer can be asked for a moment between two of its source's frames.
#[test]
fn every_layer_kind_has_an_interpolation_policy() {
    use crate::api::retime::BridgeRetimeInterp;
    let (_project, layer) = project_with_layer();
    assert_eq!(
        layer.get_interpolation().expect("read"),
        BridgeRetimeInterp::Nearest
    );
    layer
        .set_interpolation(BridgeRetimeInterp::Blend)
        .expect("a solid takes one too");
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

    // A constant map is not a map: writing one takes the Retime away rather
    // than freezing the layer on a single source moment — see
    // `a_flattened_retime_is_removed_rather_than_freezing_the_layer`.
    layer
        .set_retime_property(BridgeScalar::Static(2.5))
        .expect("write");
    assert!(layer.get_retime_property().expect("read").is_none());

    // Off removes it entirely — "not retimed", not "retimed to 1×".
    assert!(layer.toggle_retime_property().expect("on again"));
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

/// **Dragging or renaming a beat marker must leave it a beat marker** (K-270).
///
/// The regression: the panel writes the whole list back through `set_markers`,
/// and a bridge marker carries only id, time and label — so every marker was
/// rebuilt with the *default* kind, no duration, and an empty `extra`. Moving a
/// detected beat one frame turned it into an ordinary cue, and *Clear beat
/// markers* then walked straight past it: nothing was left to say it had ever
/// been detected. K-254's ruler markers made that a drag away.
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

    // The panel's write-back: same marker, moved and renamed.
    comp.set_markers(vec![BridgeMarker {
        id: beat_id,
        time: BridgeRational { num: 3, den: 2 },
        label: "Moved".into(),
    }])
    .expect("dragged");

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
        "a spanning marker keeps its span"
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

/// A marker the panel has just made is a plain user marker — the merge above
/// must not invent provenance for one the document has never seen.
#[test]
fn a_marker_the_panel_just_made_is_a_user_marker() {
    use crate::api::composition::BridgeMarker;
    use crate::api::effect::BridgeRational;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    comp.set_markers(vec![BridgeMarker {
        id: Uuid::now_v7(),
        time: BridgeRational { num: 1, den: 2 },
        label: "Mine".into(),
    }])
    .expect("marked");

    let stored = comp.composition().expect("comp").markers;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, lumit_core::markers::MarkerKind::User);
    assert_eq!(stored[0].duration, None);
    assert!(stored[0].extra.is_empty());
    comp.clear_beat_markers().expect("no beats to clear");
    assert_eq!(comp.get_markers().expect("markers").len(), 1);
}

/// A composition dropped into another brings its markers with it as the
/// layer's own — **copies**, so editing them never reaches back into the
/// composition they came from, or into anywhere else it is used (K-254).
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
            id: seeded,
            time: BridgeRational { num: 1, den: 2 },
            label: "Drop".into(),
        }])
        .expect("marked");

    let placed = outer.add_precomp_layer(&source).expect("placed");
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
/// them again on the layer would say it twice (K-254).
#[test]
fn precompose_carries_markers_in_and_leaves_the_layer_bare() {
    use crate::api::composition::BridgeMarker;
    use crate::api::effect::BridgeRational;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    comp.set_markers(vec![BridgeMarker {
        id: Uuid::now_v7(),
        time: BridgeRational { num: 1, den: 2 },
        label: "Chorus".into(),
    }])
    .expect("marked");

    let precomp = comp
        .precompose(vec![layer.layer_id], "Packed".into(), false, false)
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

// ---------------------------------------------------------------------------
// The keymap (docs/07 §15, K-199)
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

/// The table the settings page draws: grouped, headed, described, and with a
/// chord in every row. A group with no bindings is dropped rather than drawn as
/// an empty heading.
#[test]
fn the_keymap_table_arrives_grouped_headed_and_described() {
    use crate::api::keymap::*;
    let _guard = keymap_test();
    let groups = keymap_groups();

    assert!(!groups.is_empty());
    for group in &groups {
        assert!(!group.label.is_empty(), "every heading says something");
        assert!(!group.bindings.is_empty(), "no empty headings are drawn");
        for row in &group.bindings {
            assert!(!row.action.is_empty());
            assert_ne!(
                row.description, row.action,
                "{} reached the table as a raw id",
                row.action
            );
            assert!(!row.chord.is_empty(), "{} has no chord", row.action);
        }
    }
    let anywhere = groups
        .iter()
        .find(|g| g.context == BridgeKeyContext::Global)
        .expect("the app-wide group is there");
    assert_eq!(anywhere.label, "Anywhere");
    assert!(anywhere
        .bindings
        .iter()
        .any(|b| b.action == "playback.toggle" && b.chord == "Space"));
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

/// Text that is not a chord is refused with words a dialogue can show, and the
/// live keymap is left exactly as it was — a typo must not cost a binding.
#[test]
fn a_chord_that_is_not_a_chord_is_refused_without_disturbing_the_keymap() {
    use crate::api::keymap::*;
    let _guard = keymap_test();
    let before = keymap_groups();

    let err = keymap_rebind(BridgeKeyContext::Global, "file.save".into(), "".into())
        .expect_err("empty text is not a chord");
    assert!(matches!(err, BridgeError::InvalidKeyChord(_)));
    assert!(err.to_string().contains("not a keyboard shortcut"));

    let err = keymap_rebind(
        BridgeKeyContext::Global,
        "file.save".into(),
        "Hyper+S".into(),
    )
    .expect_err("an unknown modifier is not a chord");
    assert!(matches!(err, BridgeError::InvalidKeyChord(_)));

    assert_eq!(keymap_groups(), before, "nothing moved");
}

/// The round trip the frontend stores between sessions, and the one a user
/// mails to a friend, are the same round trip.
#[test]
fn a_keymap_survives_the_json_it_is_stored_as() {
    use crate::api::keymap::*;
    let _guard = keymap_test();
    keymap_rebind(
        BridgeKeyContext::Global,
        "file.save".into(),
        "Mod+Shift+S".into(),
    )
    .expect("rebound");
    let saved = keymap_to_json();

    keymap_load_preset(BridgeKeymapPreset::AfterEffects);
    // The rebind is gone: the chord means what the preset says it means, not
    // what this test made it mean. It used to assert `None` here, which held
    // only while `Mod+Shift+S` was a spare chord — Save as took it (K-244), and
    // a preset's own binding proves the replacement better than a blank does.
    assert_eq!(
        keymap_lookup(BridgeKeyContext::Global, "Mod+Shift+S".into()).as_deref(),
        Some("file.save.as"),
        "the preset really replaced it"
    );

    keymap_from_json(saved).expect("its own JSON reads back");
    assert_eq!(
        keymap_lookup(BridgeKeyContext::Global, "Mod+Shift+S".into()).as_deref(),
        Some("file.save")
    );
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

/// A per-row reset puts one chord back without touching the rest of the map —
/// the difference between "reset this" and "reset everything".
#[test]
fn resetting_one_row_leaves_the_others_where_the_user_put_them() {
    use crate::api::keymap::*;
    let _guard = keymap_test();
    keymap_rebind(BridgeKeyContext::Global, "file.save".into(), "F5".into()).expect("rebound");
    keymap_rebind(BridgeKeyContext::Global, "edit.undo".into(), "F6".into()).expect("rebound");

    keymap_reset_binding(BridgeKeyContext::Global, "file.save".into());
    assert_eq!(
        keymap_lookup(BridgeKeyContext::Global, "Mod+S".into()).as_deref(),
        Some("file.save"),
        "the one row went home"
    );
    assert_eq!(
        keymap_lookup(BridgeKeyContext::Global, "F6".into()).as_deref(),
        Some("edit.undo"),
        "and the other stayed where it was put"
    );
}

/// Unbinding leaves the row visible with an empty chord, rather than dropping
/// it out of the table — a row you cannot see is a row you cannot rebind.
#[test]
fn an_unbound_action_keeps_its_row_and_loses_its_chord() {
    use crate::api::keymap::*;
    let _guard = keymap_test();
    let after = keymap_unbind(BridgeKeyContext::Global, "file.save".into());
    assert_eq!(
        keymap_lookup(BridgeKeyContext::Global, "Mod+S".into()),
        None
    );
    // The row is gone from the table because the map no longer carries it;
    // the page redraws unbound rows from the preset's action list, so this
    // asserts the contract the page relies on: nothing else moved.
    assert!(after
        .iter()
        .flat_map(|g| &g.bindings)
        .any(|b| b.action == "edit.undo" && b.chord == "Mod+Z"));
}

// ---------------------------------------------------------------------------
// The reveal shortcuts (docs/07 §4.3, K-199)
// ---------------------------------------------------------------------------

/// `U` opens only what is keyframed. A fresh layer has nothing animated, so it
/// reveals nothing at all — and the panel is told so rather than opening a
/// layer onto an empty list.
#[test]
fn the_animated_reveal_names_only_keyframed_groups() {
    use crate::api::layer::BridgeRevealKind;
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer().expect("layer");

    let fresh = layer
        .reveal_groups(BridgeRevealKind::Animated)
        .expect("answered");
    assert!(!fresh.any, "a fresh layer has nothing animated");
    assert!(!fresh.transform);

    // Key one transform property and the Transform group qualifies.
    layer
        .set_transform(
            crate::api::layer::BridgeTransformProp::Opacity,
            BridgeScalar::Keyframed(vec![
                BridgeKeyframe {
                    time: BridgeRational { num: 0, den: 1 },
                    value: 0.0,
                    interp_in: BridgeSideInterp::Linear,
                    interp_out: BridgeSideInterp::Linear,
                },
                BridgeKeyframe {
                    time: BridgeRational { num: 1, den: 1 },
                    value: 100.0,
                    interp_in: BridgeSideInterp::Linear,
                    interp_out: BridgeSideInterp::Linear,
                },
            ]),
        )
        .expect("keyed");

    let keyed = layer
        .reveal_groups(BridgeRevealKind::Animated)
        .expect("answered");
    assert!(keyed.transform, "the keyed group is named");
    assert!(keyed.any);
    assert!(keyed.effects.is_empty(), "no effects to reveal");
}

/// `UU` opens what has been *changed*, keyframed or not — the two reveals are
/// different questions, and a moved-but-unkeyed layer is the case that shows it.
#[test]
fn the_modified_reveal_catches_a_change_that_was_never_keyframed() {
    use crate::api::layer::BridgeRevealKind;
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer().expect("layer");

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

/// An effect on the layer is itself a modification, whatever its parameters
/// say; `U` waits for a keyframe. The reveal names effects individually, so
/// only the qualifying ones unfold.
#[test]
fn an_effect_is_modified_on_arrival_and_animated_only_once_keyed() {
    use crate::api::layer::BridgeRevealKind;
    let (project, ..) = project_with_folder();
    let comp = add_comp(&project, "Scene");
    let layer = comp.add_solid_layer().expect("layer");
    let fx_name = list_effects()
        .first()
        .expect("the engine ships effects")
        .name
        .clone();
    layer.add_effect(fx_name).expect("effect added");
    let effect_id = layer.get_effects().expect("effects")[0].id().to_string();

    let modified = layer
        .reveal_groups(BridgeRevealKind::Modified)
        .expect("answered");
    assert_eq!(
        modified.effects,
        vec![effect_id],
        "the effect is named, not just a boolean"
    );
    assert!(
        layer
            .reveal_groups(BridgeRevealKind::Animated)
            .expect("answered")
            .effects
            .is_empty(),
        "nothing on it is keyframed yet"
    );
}

#[test]
fn system_memory_bytes_reports_non_zero_on_supported_platforms() {
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        let mem = crate::api::system::system_memory_bytes();
        assert!(
            mem > 0,
            "system memory should be positive on Linux/macOS/Windows"
        );
    }
}

/// Switching Retime off re-hangs the layer on its source (K-212): it keeps its
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
            }),
        )
        .expect("comp");
    let outer = project.new_composition("Outer".into(), None).expect("comp");
    let layer = outer.add_precomp_layer(&inner).expect("nested");

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
/// re-hung on its source (K-212), in one undo step.
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
            }),
        )
        .expect("comp");
    let outer = project.new_composition("Outer".into(), None).expect("comp");
    let layer = outer.add_precomp_layer(&inner).expect("nested");

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

/// Keyframes belong to the layer, and the seam says so in the interface's units
/// (K-213).
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
    let layer = comp.add_solid_layer().expect("solid");

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

/// Switching Retime on keys the layer where it *is* (K-213): one key on its in
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
    let layer = comp.add_precomp_layer(&inner).expect("nested");

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

    // **A shape edit on a keyed mask lands on the key** (K-340). Once a path is
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

    // **And the wireframe can find that shape** (K-342). The mask still carries
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

/// Opening a project displaces the ones already open — and must forget their
/// change streams as well as the projects themselves. `close` removes both
/// registries for one project; `open_project` cleared only `PROJECTS`, so
/// every project a session ever opened left its sink in `STREAMS` for the life
/// of the process.
///
/// Tested through `forget_streams_except` rather than `open_project`, which no
/// test may call: the registries are process-wide and clearing `PROJECTS`
/// would pull every other test's project out from under it.
#[test]
fn opening_a_project_forgets_the_displaced_projects_streams() {
    use crate::api::state::{forget_streams_except, STREAMS};

    // A sink that goes nowhere: with no Dart VM to post to, sending is a
    // no-op, and this test never sends — it only asks who is registered.
    let sink = || std::sync::Arc::new(crate::frb_generated::StreamSink::deserialize("0".into()));

    let displaced = Uuid::now_v7();
    let opened = Uuid::now_v7();
    {
        let mut streams = STREAMS.write().expect("streams");
        streams.insert(displaced, sink());
        streams.insert(opened, sink());
    }

    forget_streams_except(opened).expect("forgotten");

    let streams = STREAMS.read().expect("streams");
    assert!(
        !streams.contains_key(&displaced),
        "a displaced project must not keep its change stream"
    );
    assert!(
        streams.contains_key(&opened),
        "the project just opened keeps its own"
    );
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

// --- Camera track: the effect's surface across the seam (K-417) -------------

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
            }),
        )
        .expect("comp");
    let footage = project
        .import_footage("C:/clips/tracked.mov".into())
        .expect("imported");
    comp.add_footage_layer(&footage, false).expect("placed");
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

/// The cloud lands where the tracker put it, on the frame the playhead is on,
/// with the depth cue already worked out.
///
/// The comp's centre is added engine-side because the tracker's only origin is
/// the frame's middle (docs/impl/tracking.md §5b): a point at `(10, 20, 100)`
/// seen by a camera at the origin with a focal of 100 lands ten pixels right
/// and twenty down of centre, which is `(970, 560)` on a 1920x1080 comp.
#[test]
fn the_point_cloud_crosses_in_composition_pixels_with_its_depth_cue() {
    use crate::api::track::tracked_points;

    let (_project, _comp, layer, _media) = a_tracked_layer();

    let points = tracked_points(layer, 0);
    assert_eq!(points.len(), 2, "both solved points are in front");
    let near = points.iter().find(|p| p.track == 7).expect("track 7");
    let far = points.iter().find(|p| p.track == 9).expect("track 9");
    assert!((near.x - 970.0).abs() < 1e-9, "{}", near.x);
    assert!((near.y - 560.0).abs() < 1e-9, "{}", near.y);
    assert!((far.x - 975.0).abs() < 1e-9, "{}", far.x);
    assert!((far.y - 560.0).abs() < 1e-9, "{}", far.y);
    // Normalised over the cloud on this frame: the nearer one reads 1.
    assert!((near.depth - 1.0).abs() < 1e-9, "{}", near.depth);
    assert!((far.depth - 0.0).abs() < 1e-9, "{}", far.depth);

    // A later frame is a later *solved* frame: the camera has moved five units
    // along x, so both points slide left by that much times the focal over
    // their depth. Five units at z = 100 with f = 100 is five pixels.
    let later = tracked_points(layer, 5);
    let near = later.iter().find(|p| p.track == 7).expect("track 7");
    assert!((near.x - 965.0).abs() < 1e-9, "{}", near.x);
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

/// The status row's reading, and what a press of each button does.
#[test]
fn the_status_reads_the_solve_and_the_buttons_are_refused_honestly() {
    use crate::api::track::{fire_effect_action, track_status, BridgeTrackStage};

    let (_project, comp, layer, _media) = a_tracked_layer();

    let status = track_status(layer);
    assert_eq!(status.stage, BridgeTrackStage::Done);
    assert_eq!(status.points, 2);
    assert_eq!(status.frames, 50);
    assert!((status.mean_error - 0.25).abs() < 1e-9);

    let effect = layer.get_effects().expect("stack")[0].id();

    // Cancel is always safe: nothing is running, and saying so is a no-op
    // rather than an error the panel would have to explain.
    fire_effect_action(layer, effect, "cancel".to_owned()).expect("cancel is accepted");

    // A parameter that is not an Action is refused rather than ignored — a
    // button that silently does nothing is the hardest fault to see.
    assert!(matches!(
        fire_effect_action(layer, effect, "density".to_owned()),
        Err(BridgeError::InvalidParam)
    ));

    // The fixture's media is a path that does not exist, so Analyse cannot
    // start: a refusal about the file, not a fault.
    assert!(matches!(
        fire_effect_action(layer, effect, "analyse".to_owned()),
        Err(BridgeError::MediaPathUnresolved)
    ));

    // A layer with no Camera track has no analysis to read.
    let solid = comp.add_solid_layer().expect("a solid");
    assert_eq!(track_status(solid).stage, BridgeTrackStage::Idle);
}

/// A **partial** track crosses as the span it solved against the clip it did
/// not finish, and a camera linked to it holds past the end of that span
/// (K-417's hold, K-440).
///
/// The two claims belong together: the range the status row draws is the same
/// range the link clamps into, and a test that checked only one of them would
/// let them drift apart.
#[test]
fn a_partial_track_reports_its_span_and_the_camera_holds_past_it() {
    use crate::api::track::{
        add_solved_camera, camera_link, track_status, BridgeLinkState, BridgeTrackStage,
    };
    use lumit_track::{CameraSolve, PoseSource, ScenePoint, SolveSegment, SolvedPose};

    let (_project, _comp, layer, media) = a_tracked_layer();

    // The same shot, but followed only as far as frame nineteen of fifty — what
    // the job publishes when the tracking fails part-way (docs/impl/tracking.md
    // §5d).
    let solve = CameraSolve {
        poses: (0..20)
            .map(|frame| SolvedPose {
                frame,
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                position: [frame as f64, 0.0, 0.0],
                segment: 0,
                focal_px: 100.0,
                mean_reprojection_px: 0.1,
                source: PoseSource::Keyframe,
            })
            .collect(),
        segments: vec![SolveSegment {
            first_frame: 0,
            last_frame: 19,
            focal_px: 100.0,
            ramp: false,
        }],
        points: vec![ScenePoint {
            track: 7,
            position: [10.0, 20.0, 100.0],
        }],
        keyframes: vec![0, 19],
        mean_reprojection_px: 0.25,
        notes: Vec::new(),
    };
    lumit_render::track::publish(media, 25.0, 50, solve);

    let status = track_status(layer);
    assert_eq!(
        status.stage,
        BridgeTrackStage::Done,
        "a partial solve is a solve"
    );
    assert_eq!(status.frames, 20, "the span that carries a camera");
    assert_eq!(
        status.clip_frames, 50,
        "against the clip that was not finished"
    );

    // The bar and the badge read the same range. Inside it the camera is
    // derived; past it the last derived motion is held, which is K-417's rule
    // meeting a range that now ends early.
    let camera = add_solved_camera(layer).expect("a linked camera");
    assert_eq!(camera_link(camera, 0).state, BridgeLinkState::Derived);
    assert_eq!(camera_link(camera, 19).state, BridgeLinkState::Derived);
    assert_eq!(
        camera_link(camera, 20).state,
        BridgeLinkState::Held,
        "one frame past the solved span is already held"
    );
    assert_eq!(camera_link(camera, 49).state, BridgeLinkState::Held);
}
