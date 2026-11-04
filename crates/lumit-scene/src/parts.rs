//! A model's parts: the number each goes by in a scene pass, and its name. A
//! part is a node of the file's tree, or one material of a node that has more
//! than one. The number comes from the layer and the part's names alone, so it
//! stays put between frames, between runs and when the file is saved again
//! with its nodes in another order.

use crate::ModelScene;

/// The number `key` goes by on `layer`. Twenty-four bits, as a picture's three
/// channels carry it, and never nought, which is nothing at all.
fn number(layer: &[u8; 16], key: &str) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in layer.iter().chain(key.as_bytes()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let folded = (h ^ (h >> 24) ^ (h >> 48)) as u32 & 0x00ff_ffff;
    folded.max(1)
}

/// The number a whole 3D layer goes by.
#[must_use]
pub fn layer_number(layer: &[u8; 16]) -> u32 {
    number(layer, "")
}

impl ModelScene {
    /// **The one place a model's part gets its number.** `material` is the
    /// material's place in the file, past the end for a primitive with none.
    #[must_use]
    pub fn part_number(&self, layer: &[u8; 16], node: usize, material: usize) -> u32 {
        let path = self.nodes.get(node).map_or("", |n| n.path.as_str());
        if self.node_materials(node).len() < 2 {
            return number(layer, path);
        }
        let name = self.materials.get(material).map(|m| m.name.as_str());
        match name.filter(|n| !n.is_empty()) {
            Some(name) => number(layer, &format!("{path}@{name}")),
            None => number(layer, &format!("{path}@#{material}")),
        }
    }

    /// Every part with its number on `layer` and the name to show. A node with
    /// no name goes by its mesh's, or by `unnamed` with neither, and a material
    /// with none by its place in the file.
    #[must_use]
    pub fn parts(&self, layer: &[u8; 16], unnamed: &str) -> Vec<(u32, String)> {
        let mut parts = Vec::new();
        for (k, node) in self.nodes.iter().enumerate() {
            let Some(mesh) = node.mesh.and_then(|m| self.meshes.get(m)) else {
                continue;
            };
            let shown = [node.name.as_str(), mesh.name.as_str(), unnamed]
                .into_iter()
                .find(|n| !n.is_empty())
                .unwrap_or_default();
            let materials = self.node_materials(k);
            for &m in &materials {
                let name = match self.materials.get(m).map(|m| m.name.as_str()) {
                    _ if materials.len() < 2 => shown.to_owned(),
                    Some(material) if !material.is_empty() => format!("{shown} · {material}"),
                    _ => format!("{shown} · {}", m + 1),
                };
                parts.push((self.part_number(layer, k, m), name));
            }
        }
        parts
    }

    /// The materials node `k`'s mesh draws with, each once, by their place in
    /// the file. A primitive with none, or a bad one, counts as past the end.
    fn node_materials(&self, k: usize) -> Vec<usize> {
        let mesh = self
            .nodes
            .get(k)
            .and_then(|n| self.meshes.get(n.mesh?))
            .map_or(&[][..], |m| &m.primitives);
        let none = self.materials.len();
        let mut seen = Vec::new();
        for p in mesh {
            let m = p.material.filter(|&m| m < none).unwrap_or(none);
            if !seen.contains(&m) {
                seen.push(m);
            }
        }
        seen.sort_unstable();
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Material, Mesh, Node, Primitive};

    /// A matte is saved by these numbers, so a change here cuts every saved
    /// matte loose. They have to come from the names, not the place in the file.
    #[test]
    fn a_part_keeps_its_number_whatever_order_the_file_has() {
        let node = |name: &str, parent: Option<usize>, children: Vec<usize>, mesh| Node {
            name: name.into(),
            path: match parent {
                Some(_) => format!("Body/{name}"),
                None => name.into(),
            },
            parent,
            children,
            mesh,
            ..Node::default()
        };
        let drawn = |material| Primitive {
            material,
            ..Primitive::default()
        };
        let plain = Mesh {
            primitives: vec![drawn(None)],
            ..Mesh::default()
        };
        // The same model, its nodes and materials the other way round.
        let material = |name: &str| Material {
            name: name.into(),
            ..Material::default()
        };
        let one = ModelScene {
            nodes: vec![
                node("Body", None, vec![1, 2], None),
                node("Visor", Some(0), vec![], Some(0)),
                node("Strap", Some(0), vec![], Some(1)),
            ],
            roots: vec![0],
            meshes: vec![
                plain.clone(),
                Mesh {
                    primitives: vec![drawn(Some(0)), drawn(Some(1)), drawn(Some(0))],
                    ..Mesh::default()
                },
            ],
            materials: vec![material("Leather"), material("Brass")],
            ..ModelScene::default()
        };
        let other = ModelScene {
            nodes: vec![
                node("Strap", Some(2), vec![], Some(1)),
                node("Visor", Some(2), vec![], Some(0)),
                node("Body", None, vec![0, 1], None),
            ],
            roots: vec![2],
            meshes: vec![
                plain,
                Mesh {
                    primitives: vec![drawn(Some(1)), drawn(Some(0))],
                    ..Mesh::default()
                },
            ],
            materials: vec![material("Brass"), material("Leather")],
            ..ModelScene::default()
        };
        let layer = [7; 16];
        let visor = one.part_number(&layer, 1, 2);
        assert_eq!(visor, 0x0050_b7f2, "a node with one material");
        assert_eq!(visor, other.part_number(&layer, 1, 2));
        assert_ne!(visor, one.part_number(&[8; 16], 1, 2));

        let leather = one.part_number(&layer, 2, 0);
        assert_eq!(leather, 0x00e3_4f36, "a node with two materials");
        assert_eq!(leather, other.part_number(&layer, 0, 1));
        assert_ne!(leather, one.part_number(&layer, 2, 1));

        let names: Vec<String> = one
            .parts(&layer, "Layer")
            .into_iter()
            .map(|p| p.1)
            .collect();
        assert_eq!(names, ["Visor", "Strap · Leather", "Strap · Brass"]);
    }
}
