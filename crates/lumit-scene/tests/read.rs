//! A tiny glTF and a tiny OBJ open, so a model file never silently fails.

#![allow(clippy::unwrap_used)]

use std::sync::atomic::AtomicBool;

use base64::Engine;
use lumit_ingress::Limits;

fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lumit-scene-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_gltf_triangle_opens() {
    let floats: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0,"translation":[0,0,-2]}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],
        "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
            "min":[0,0,0],"max":[1,1,0]}}],
        "bufferViews":[{{"buffer":0,"byteLength":36}}],
        "buffers":[{{"byteLength":36,"uri":"data:application/octet-stream;base64,{data}"}}]}}"#
    );
    let dir = folder("gltf");
    let path = dir.join("triangle.gltf");
    std::fs::write(&path, json).unwrap();

    let scene = lumit_scene::read(&path, Limits::MODEL, &dir, &AtomicBool::new(false)).unwrap();
    assert_eq!(scene.roots, vec![0]);
    assert_eq!(scene.meshes[0].primitives[0].indices, vec![0, 1, 2]);
    assert_eq!(scene.bounds.min, [0.0, 0.0, -2.0]);
    assert_eq!(scene.bounds.max, [1.0, 1.0, -2.0]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_obj_finds_its_mtl_by_name_and_says_what_is_missing() {
    let dir = folder("obj");
    let model = dir.join("model");
    let mats = dir.join("mats");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::create_dir_all(&mats).unwrap();
    // The MTL is named by a path from another machine, and sits in a folder
    // beside the model's.
    std::fs::write(
        model.join("quad.obj"),
        "mtllib C:\\Users\\someone\\quad.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nusemtl red\nf 1 2 3 4\nusemtl plain\nf 1 3 4\n",
    )
    .unwrap();
    std::fs::write(
        mats.join("quad.mtl"),
        "newmtl red\nKd 1 0 0\nmap_Kd paint.png\nnewmtl plain\nmap_Kd plain.png\n",
    )
    .unwrap();
    let path = model.join("quad.obj");

    let scene = lumit_scene::read(&path, Limits::MODEL, &dir, &AtomicBool::new(false)).unwrap();
    let triangles: usize = scene
        .meshes
        .iter()
        .flat_map(|m| &m.primitives)
        .map(|p| p.indices.len() / 3)
        .sum();
    assert_eq!(triangles, 3);
    let colour = |name: &str| {
        let m = scene.materials.iter().find(|m| m.name == name).unwrap();
        m.base_colour[..3].to_vec()
    };
    assert_eq!(colour("red"), [1.0, 0.0, 0.0]);
    // A picture with no colour shows as it is, not black.
    assert_eq!(colour("plain"), [1.0, 1.0, 1.0]);
    assert_eq!(scene.missing, ["paint.png", "plain.png"]);
    assert_eq!(lumit_scene::dependants(&path), vec![mats.join("quad.mtl")]);
    std::fs::remove_dir_all(dir).unwrap();
}
