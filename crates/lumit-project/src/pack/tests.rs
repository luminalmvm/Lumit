use super::*;
use lumit_core::model::{
    packed_file_id, packed_id, EffectValue, FileParam, FootageItem, ProjectItem, ProxyRef,
};

fn project_with(dir: &Path, name: &str, bytes: &[u8]) -> (Document, Uuid, PathBuf) {
    let file = dir.join("footage").join(name);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, bytes).unwrap();
    let id = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: None,
        id,
        name: name.into(),
        extra: serde_json::Map::new(),
        colour_space: None,
        media: MediaRef {
            relative_path: format!("footage/{name}"),
            absolute_path: file.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        source_layer: None,
        interpretation: Default::default(),
    }));
    (doc, id, file)
}

fn source(item: Uuid, file: &Path) -> PackSource {
    PackSource {
        item,
        files: vec![file.to_path_buf()],
    }
}

fn file_ref(dir: &Path, relative: &str, bytes: &[u8]) -> MediaRef {
    let file = dir.join(relative);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, bytes).unwrap();
    MediaRef {
        relative_path: relative.into(),
        absolute_path: file.to_string_lossy().into_owned(),
        fingerprint: None,
        extra: serde_json::Map::new(),
    }
}

#[test]
fn packed_footage_outlives_its_original() {
    let dir = tempfile::tempdir().unwrap();
    let bytes: Vec<u8> = (0..300_000u32).map(|n| (n % 251) as u8).collect();
    let (mut doc, id, _) = project_with(dir.path(), "clip.mov", &bytes);
    let (proxy_id, config_id) = (packed_id(id), packed_id(doc.id));
    doc.proxies.insert(
        id,
        ProxyRef {
            media: file_ref(dir.path(), "footage/clip_proxy.mov", b"small picture"),
            enabled: true,
            extra: serde_json::Map::new(),
        },
    );
    doc.colour.config = Some(file_ref(dir.path(), "ocio/config.ocio", b"a config"));
    file_ref(dir.path(), "ocio/luts/grade.cube", b"a table");
    let look = file_ref(dir.path(), "looks/look.cube", b"a look").absolute_path;
    let mut lut = lumit_core::fx::builtins::instantiate("lut").unwrap();
    let look_id = packed_file_id(lut.id, "file", 0);
    let slot = lut.params.iter_mut().find(|p| p.id == "file").unwrap();
    slot.value = EffectValue::File(FileParam::single(look.clone()));
    let stress = crate::fixtures::stress_document(&crate::fixtures::StressParams::TINY);
    let comp = stress.items.into_iter().find_map(|item| match item {
        ProjectItem::Composition(comp) => Some(comp),
        _ => None,
    });
    let mut comp = comp.unwrap();
    comp.layers[0].effects.push(lut);
    doc.items.push(ProjectItem::Composition(comp));
    let offered = |doc: &Document| {
        let at = |id| PathBuf::from(doc.packed_path(id).unwrap());
        let table = at(config_id).with_file_name("luts").join("grade.cube");
        vec![
            source(id, &at(id)),
            source(proxy_id, &at(proxy_id)),
            source(look_id, &at(look_id)),
            PackSource {
                item: config_id,
                files: vec![at(config_id), table],
            },
        ]
    };
    let lum = dir.path().join("scene.lum");

    let first = save_packed(&doc, &lum, None, &offered(&doc), &mut |_, _| true).unwrap();
    assert_eq!(first.files, 5);
    fs::remove_dir_all(dir.path().join("footage")).unwrap();
    fs::remove_dir_all(dir.path().join("ocio")).unwrap();
    fs::remove_dir_all(dir.path().join("looks")).unwrap();

    let (mut opened, _) = crate::open(&lum).unwrap();
    let cache = dir.path().join("cache");
    let restored = restore_packed(
        &mut opened,
        std::slice::from_ref(&lum),
        dir.path(),
        &cache,
        &mut |_, _| true,
    )
    .unwrap();
    assert_eq!(restored, (4, Vec::new()));
    let ProjectItem::Footage(f) = opened.item(id).unwrap() else {
        panic!("the footage item");
    };
    let copy = PathBuf::from(&f.media.absolute_path);
    assert_eq!(fs::read(&copy).unwrap(), bytes);
    assert!(Path::new(opened.packed_path(look_id).unwrap()).starts_with(&cache));
    let mut written = opened.clone();
    keep_original_paths(&mut written, &cache);
    assert_eq!(written.packed_path(look_id), Some(look.as_str()));

    let again = save_packed(&opened, &lum, Some(&lum), &offered(&opened), &mut |_, _| {
        true
    })
    .unwrap();
    assert_eq!(again.packed.len(), 4);
    fs::remove_dir_all(&cache).unwrap();
    let (mut reopened, _) = crate::open(&lum).unwrap();
    restore_packed(
        &mut reopened,
        std::slice::from_ref(&lum),
        dir.path(),
        &cache,
        &mut |_, _| true,
    )
    .unwrap();
    let ProjectItem::Footage(f) = reopened.item(id).unwrap() else {
        panic!("the footage item");
    };
    assert_eq!(fs::read(&f.media.absolute_path).unwrap(), bytes);
    let proxy = &reopened.packed_ref(proxy_id).unwrap().absolute_path;
    assert_eq!(fs::read(proxy).unwrap(), b"small picture");
    let config = Path::new(&reopened.packed_ref(config_id).unwrap().absolute_path);
    assert_eq!(fs::read(config).unwrap(), b"a config");
    let table = config.with_file_name("luts").join("grade.cube");
    assert_eq!(fs::read(table).unwrap(), b"a table");
    let look = reopened.packed_path(look_id).unwrap();
    assert_eq!(fs::read(look).unwrap(), b"a look");

    let unpacked = unpack(&reopened, &[lum], dir.path(), &cache, &mut |_, _| true).unwrap();
    assert!(unpacked.kept.is_empty());
    let moved: HashMap<Uuid, MediaRef> = unpacked.moved.into_iter().collect();
    assert_eq!(moved[&id].relative_path, "media/clip.mov");
    assert_eq!(moved[&proxy_id].relative_path, "media/clip_proxy.mov");
    assert_eq!(moved[&config_id].relative_path, "media/config/config.ocio");
    assert_eq!(moved[&look_id].relative_path, "media/look.cube");
    let media = dir.path().join("media");
    assert_eq!(fs::read(media.join("clip.mov")).unwrap(), bytes);
    assert_eq!(
        fs::read(media.join("config/luts/grade.cube")).unwrap(),
        b"a table"
    );

    let root = dir.path().join("packed");
    let [held, old, unmarked, other] = [(); 4].map(|()| Uuid::now_v7());
    let folder = |id: Uuid| root.join(id.to_string());
    let holding = hold_read_out(&root, held, true).unwrap();
    drop(hold_read_out(&root, old, true));
    fs::create_dir_all(folder(unmarked)).unwrap();
    let long_ago = SystemTime::now() - IDLE - Duration::from_secs(60);
    let marker = File::options().write(true).open(folder(old).join(IN_USE));
    marker.unwrap().set_modified(long_ago).unwrap();
    holding.set_modified(long_ago).unwrap();
    drop(hold_read_out(&root, other, true));
    assert!(folder(held).is_dir() && !folder(old).exists());
    drop(holding);
    drop(hold_read_out(&root, old, false));
    assert!(!folder(held).exists());
    assert!(folder(unmarked).is_dir() && folder(other).is_dir());
}

#[test]
fn a_cancelled_pack_leaves_the_project_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = vec![7u8; CHUNK * 3];
    let (doc, id, original) = project_with(dir.path(), "clip.mov", &bytes);
    let lum = dir.path().join("scene.lum");
    crate::save(&doc, &lum).unwrap();
    let before = fs::read(&lum).unwrap();

    let stopped = save_packed(
        &doc,
        &lum,
        Some(&lum),
        &[source(id, &original)],
        &mut |done, _| done < CHUNK as u64 * 2,
    );
    assert!(matches!(stopped, Err(ProjectError::Cancelled)));
    assert_eq!(fs::read(&lum).unwrap(), before);
    let left: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(left.is_empty(), "no half-written file is left behind");
}

#[test]
fn an_entry_that_points_outside_is_never_written() {
    let dir = tempfile::tempdir().unwrap();
    let (mut doc, id, original) = project_with(dir.path(), "clip.mov", b"picture");
    let fingerprint = fingerprint_path(&original).unwrap();
    fs::remove_file(&original).unwrap();
    doc.packed.insert(
        id,
        PackedMedia {
            entry: "media/../../escaped.mov".into(),
            fingerprint: fingerprint.clone(),
            original: None,
            extra: serde_json::Map::new(),
        },
    );
    let lum = dir.path().join("crafted.lum");
    let mut zip = ZipWriter::new(File::create(&lum).unwrap());
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("media/../../escaped.mov", stored).unwrap();
    zip.write_all(b"picture").unwrap();
    zip.finish().unwrap();

    let cache = dir.path().join("a").join("b").join("cache");
    let restored = restore_packed(&mut doc, &[lum], dir.path(), &cache, &mut |_, _| true).unwrap();
    assert_eq!(restored.0, 0);
    assert!(!dir.path().join("a").join("escaped.mov").exists());
    assert!(!dir.path().join("a").join("b").join("escaped.mov").exists());

    let lum = dir.path().join("listed.lum");
    let mut zip = ZipWriter::new(File::create(&lum).unwrap());
    zip.start_file("media/held/other.mov", stored).unwrap();
    zip.write_all(b"another picture").unwrap();
    zip.finish().unwrap();
    for (folder, stays) in [("gone", false), ("held", true)] {
        let entry = PackedMedia {
            entry: format!("media/{folder}/clip.mov"),
            fingerprint: fingerprint.clone(),
            original: None,
            extra: serde_json::Map::new(),
        };
        doc.packed.insert(id, entry);
        let archive = std::slice::from_ref(&lum);
        let (_, unplaced) =
            restore_packed(&mut doc, archive, dir.path(), &cache, &mut |_, _| true).unwrap();
        assert_eq!(doc.packed.contains_key(&id), stays, "{folder}");
        assert_eq!(unplaced == [id], stays, "{folder}");
    }
}
