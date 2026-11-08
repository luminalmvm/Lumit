//! What a scene works out once it has been read, whichever file it came from:
//! a name each part and take can be found by, the boxes a pose grows from, and
//! a refusal for skins that don't add up.

use std::collections::HashMap;

use glam::{Mat4, Vec3};

use crate::{Bounds, ModelScene};

/// Everything a pose reads. Answers what it found broken, as names for the
/// missing list. A node whose skin is broken loses it and draws at rest.
pub fn settle(scene: &mut ModelScene) -> Vec<String> {
    name_takes(scene);
    name_parts(scene);
    let broken = check_skins(scene);
    measure(scene);
    broken
}

/// A take with no name is numbered, and one sharing a name gets a number too,
/// since the layer keeps its take by name.
fn name_takes(scene: &mut ModelScene) {
    let mut seen: HashMap<String, u32> = HashMap::new();
    for (i, clip) in scene.clips.iter_mut().enumerate() {
        if clip.name.is_empty() {
            clip.name = format!("Take {}", i + 1);
        }
        let times = seen.entry(clip.name.clone()).or_insert(0);
        *times += 1;
        if *times > 1 {
            clip.name = format!("{} {times}", clip.name);
        }
    }
}

/// Each node's path from its root by name, with a number for one with no name
/// or sharing it with a sister, so a layer's rows find their part again when
/// the file's nodes move about.
fn name_parts(scene: &mut ModelScene) {
    let mut stack: Vec<(usize, String)> = Vec::new();
    let tag = |seen: &mut HashMap<String, u32>, name: &str, k: usize| {
        if name.is_empty() {
            return format!("#{k}");
        }
        let times = seen.entry(name.to_owned()).or_insert(0);
        *times += 1;
        if *times > 1 {
            format!("{name}#{times}")
        } else {
            name.to_owned()
        }
    };
    let mut seen = HashMap::new();
    for (k, &root) in scene.roots.iter().enumerate() {
        let name = scene.nodes.get(root).map_or("", |n| n.name.as_str());
        stack.push((root, tag(&mut seen, name, k)));
    }
    while let Some((n, path)) = stack.pop() {
        let Some(node) = scene.nodes.get(n) else {
            continue;
        };
        let mut seen = HashMap::new();
        for (k, &c) in node.children.iter().enumerate() {
            let child = scene.nodes.get(c).filter(|ch| ch.parent == Some(n));
            if let Some(child) = child {
                stack.push((c, format!("{path}/{}", tag(&mut seen, &child.name, k))));
            }
        }
        if let Some(node) = scene.nodes.get_mut(n) {
            node.path = path;
        }
    }
}

/// Take away every skin that can't be drawn: one naming a joint that isn't
/// there, with a matrix that won't invert or with a vertex pointing past its
/// joints. Its node draws at rest.
fn check_skins(scene: &mut ModelScene) -> Vec<String> {
    let skins = std::mem::take(&mut scene.skins);
    let meshes = std::mem::take(&mut scene.meshes);
    let count = scene.nodes.len();
    let mut broken = Vec::new();
    for node in &mut scene.nodes {
        let Some(s) = node.skin else {
            continue;
        };
        let good = skins.get(s).is_some_and(|skin| {
            let joints_fit = skin.joints.len() == skin.inverse_bind.len()
                && !skin.joints.is_empty()
                && skin.joints.iter().all(|&j| j < count);
            let matrices = skin.inverse_bind.iter().all(|m| {
                let m = Mat4::from_cols_array_2d(m);
                m.is_finite() && m.determinant() != 0.0
            });
            let mesh = node.mesh.and_then(|m| meshes.get(m));
            let vertices = mesh.is_some_and(|m| {
                m.primitives.iter().all(|p| {
                    p.joints.len() == p.positions.len()
                        && p.weights.len() == p.positions.len()
                        && p.joints
                            .iter()
                            .flatten()
                            .all(|&j| usize::from(j) < skin.joints.len())
                })
            });
            joints_fit && matrices && vertices
        });
        if !good {
            node.skin = None;
            let name = match node.name.is_empty() {
                true => mesh_name(&meshes, node.mesh),
                false => &node.name,
            };
            broken.push(format!("{name} skin"));
        }
    }
    scene.skins = skins;
    scene.meshes = meshes;
    broken
}

fn mesh_name(meshes: &[crate::Mesh], mesh: Option<usize>) -> &str {
    mesh.and_then(|m| meshes.get(m)).map_or("", |m| &m.name)
}

fn extend(b: &mut Bounds, p: Vec3) {
    b.min = Vec3::from(b.min).min(p).to_array();
    b.max = Vec3::from(b.max).max(p).to_array();
}

fn empty() -> Bounds {
    Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    }
}

/// Each mesh's box, how far each morph target reaches, and the box of the
/// vertices each joint of a skin moves.
pub fn measure(scene: &mut ModelScene) {
    for mesh in &mut scene.meshes {
        let mut whole = empty();
        for p in &mut mesh.primitives {
            for &q in &p.positions {
                extend(&mut whole, q.into());
            }
            for target in &mut p.morph_targets {
                let mut reach = empty();
                for &d in &target.positions {
                    extend(&mut reach, d.into());
                }
                if reach.min[0] <= reach.max[0] {
                    target.reach = reach;
                }
            }
        }
        if whole.min[0] <= whole.max[0] {
            mesh.bounds = whole;
        }
    }
    for skin in &mut scene.skins {
        skin.boxes = vec![empty(); skin.joints.len()];
    }
    for node in &scene.nodes {
        let (Some(s), Some(m)) = (node.skin, node.mesh) else {
            continue;
        };
        let (Some(skin), Some(mesh)) = (scene.skins.get_mut(s), scene.meshes.get(m)) else {
            continue;
        };
        for p in &mesh.primitives {
            for (i, (joints, weights)) in p.joints.iter().zip(&p.weights).enumerate() {
                let Some(&q) = p.positions.get(i) else {
                    continue;
                };
                for (&j, &w) in joints.iter().zip(weights) {
                    if let Some(b) = skin.boxes.get_mut(usize::from(j)).filter(|_| w > 0.0) {
                        extend(b, q.into());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Mesh, Node, Primitive, Skin};

    fn skinned(joints: Vec<usize>, vertex_joint: u16) -> ModelScene {
        ModelScene {
            nodes: vec![
                Node {
                    name: "body".into(),
                    mesh: Some(0),
                    skin: Some(0),
                    ..Node::default()
                },
                Node::default(),
            ],
            roots: vec![0, 1],
            meshes: vec![Mesh {
                primitives: vec![Primitive {
                    positions: vec![[1.0, 2.0, 3.0]],
                    joints: vec![[vertex_joint, 0, 0, 0]],
                    weights: vec![[1.0, 0.0, 0.0, 0.0]],
                    ..Primitive::default()
                }],
                ..Mesh::default()
            }],
            skins: vec![Skin {
                inverse_bind: vec![Mat4::IDENTITY.to_cols_array_2d(); joints.len()],
                joints,
                ..Skin::default()
            }],
            ..ModelScene::default()
        }
    }

    /// A skin that is whole keeps its joint boxes; a joint that isn't there,
    /// or a vertex pointing past its joints, loses the skin and says so.
    #[test]
    fn a_broken_skin_is_taken_away_and_named() {
        let mut good = skinned(vec![1], 0);
        assert!(settle(&mut good).is_empty());
        assert_eq!(good.nodes[0].skin, Some(0));
        assert_eq!(good.skins[0].boxes[0].max, [1.0, 2.0, 3.0]);
        for mut bad in [skinned(vec![7], 0), skinned(vec![1], 3), skinned(vec![], 0)] {
            assert_eq!(settle(&mut bad), vec!["body skin".to_owned()]);
            assert_eq!(bad.nodes[0].skin, None);
        }
    }
}
