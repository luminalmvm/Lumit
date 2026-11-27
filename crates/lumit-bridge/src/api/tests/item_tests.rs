//! Project items: folders, deleting, filing, relinking and placing footage.

use super::*;

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

/// Deleting a folder takes what is inside it and the layers showing it, and
/// one undo puts them all back.
#[test]
fn deleting_a_folder_takes_its_footage_and_layers_and_one_undo_restores_them() {
    let (project, folder, filed, loose) = project_with_folder();
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let ItemReference::Footage(footage) = &filed else {
        panic!("seeded as footage")
    };
    comp.add_footage_layer(footage, false, None, None)
        .expect("placed");
    assert_eq!(folder.uses().expect("counted"), 1);

    folder.delete().expect("deleted");
    assert!(comp.get_layers().expect("layers").is_empty());
    assert!(matches!(filed.name(), Err(BridgeError::InvalidItem)));
    assert!(loose.name().is_ok(), "the loose clip stayed");

    project.undo().expect("undone");
    assert_eq!(comp.get_layers().expect("layers").len(), 1);
    assert!(filed.name().is_ok());
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
        interpretation: Default::default(),
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
    crate::test_files::remove_when_free(&clip);
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
    let (mut doc, _) = lumit_project::open(&lum).expect("opens");
    assert!(doc.packed.is_empty(), "the file no longer carries it");
    project.close().expect("closed");

    // A file from somebody else can arrive saying that every save packs. It
    // is not believed: it opens with that off, and a save packs nothing. Once
    // it is switched on here, a save packs. Opened the way a recovery opens
    // one, which leaves the projects other tests have open alone.
    doc.auto_pack = true;
    let sent = dir.path().join("sent.lum");
    lumit_project::save(&doc, &sent).expect("written");
    let open = || {
        let opened = LumitBridgeState::new_project(None).expect("a new project");
        let name = sent.to_string_lossy().into_owned();
        opened.restore_journal(name, None).expect("opens");
        opened
    };
    let packed = || lumit_project::open(&sent).expect("opens").0.packed.len();
    let received = open();
    assert!(!received.pack_state().expect("state").auto_pack);
    received.save(String::new()).expect("saved");
    assert_eq!(packed(), 0, "nothing went into the file");
    received.set_auto_pack(true).expect("switched on");
    received.save(String::new()).expect("saved");
    assert_eq!(packed(), 1);
    received.close().expect("closed");
    // And this machine's own choice is still there the next time.
    let again = open();
    assert!(again.pack_state().expect("state").auto_pack);
    again.close().expect("closed");
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
    comp.add_footage_layer(footage, false, None, None)
        .expect("placed");

    let layers = comp.get_layers().expect("layers");
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].get_name().expect("name"), "filed.mp4");
}
