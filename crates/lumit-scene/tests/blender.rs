//! A .blend opens through Blender. Passes without checking anything on a
//! machine with no Blender.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;
use std::sync::atomic::AtomicBool;

use lumit_ingress::Limits;
use lumit_scene::blender;

#[test]
fn a_blend_file_opens_through_blender() {
    let Some(found) = blender::find() else {
        eprintln!("no Blender on this machine, nothing checked");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let blend = dir.path().join("cube.blend");
    // Blender's own start-up scene: a cube, a camera and a light. Plus one
    // object hidden in the viewport, which mustn't come through.
    let save = format!(
        "import bpy; bpy.ops.mesh.primitive_monkey_add(); o = bpy.context.object; \
         o.name = 'Hidden'; o.hide_set(True); bpy.ops.wm.save_as_mainfile(filepath={:?})",
        blend.to_string_lossy()
    );
    let made = Command::new(&found.exe)
        .args(["-b", "--factory-startup", "--python-expr", &save])
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");

    let cache = dir.path().join("cache");
    let scene = lumit_scene::read(&blend, Limits::MODEL, &cache, &AtomicBool::new(false)).unwrap();
    let names: Vec<&str> = scene.nodes.iter().map(|n| n.name.as_str()).collect();
    for part in ["Cube", "Camera", "Light"] {
        assert!(names.contains(&part), "{part} missing from {names:?}");
    }
    assert!(!names.contains(&"Hidden"), "{names:?}");
    assert_eq!((scene.cameras.len(), scene.lights.len()), (1, 1));
    assert_eq!(blender::state(&blend), None);
}
