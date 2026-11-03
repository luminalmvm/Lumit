//! Models built in code, for tests and the bench: a box, a ball and a
//! chequered picture to wrap round them.

use crate::{Bounds, Image, Material, Mesh, ModelScene, Node, Primitive, Texture};

/// A box `size` metres on a side round the origin, one picture square to a
/// face.
#[must_use]
pub fn cube(size: f32, material: Option<usize>) -> Primitive {
    let h = size * 0.5;
    // Each face's normal, the way its pictures run across, and up it.
    let faces: [[f32; 9]; 6] = [
        [1., 0., 0., 0., 0., -1., 0., 1., 0.],
        [-1., 0., 0., 0., 0., 1., 0., 1., 0.],
        [0., 1., 0., 1., 0., 0., 0., 0., -1.],
        [0., -1., 0., 1., 0., 0., 0., 0., 1.],
        [0., 0., 1., 1., 0., 0., 0., 1., 0.],
        [0., 0., -1., -1., 0., 0., 0., 1., 0.],
    ];
    let mut p = Primitive {
        material,
        ..Primitive::default()
    };
    for (k, f) in faces.iter().enumerate() {
        let (n, t, b) = (&f[0..3], &f[3..6], &f[6..9]);
        for (u, v) in [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)] {
            let (x, y) = (2.0 * u - 1.0, 1.0 - 2.0 * v);
            p.positions
                .push([0, 1, 2].map(|i| (n[i] + t[i] * x + b[i] * y) * h));
            p.normals.push([n[0], n[1], n[2]]);
            p.tangents.push([t[0], t[1], t[2], 1.0]);
            p.uv0.push([u, v]);
        }
        let o = 4 * k as u32;
        p.indices.extend([o, o + 1, o + 2, o, o + 2, o + 3]);
    }
    p
}

/// A ball `radius` metres round the origin, `rings` from pole to pole and
/// `segments` round, so twice their product triangles.
#[must_use]
pub fn sphere(radius: f32, segments: u32, rings: u32, material: Option<usize>) -> Primitive {
    let (segments, rings) = (segments.max(3), rings.max(2));
    let mut p = Primitive {
        material,
        ..Primitive::default()
    };
    for i in 0..=rings {
        let v = i as f32 / rings as f32;
        let (st, ct) = (v * std::f32::consts::PI).sin_cos();
        for j in 0..=segments {
            let u = j as f32 / segments as f32;
            let (sp, cp) = (u * std::f32::consts::TAU).sin_cos();
            let n = [st * cp, ct, -st * sp];
            p.positions.push(n.map(|c| c * radius));
            p.normals.push(n);
            p.tangents.push([-sp, 0.0, -cp, 1.0]);
            p.uv0.push([u, v]);
        }
    }
    let row = segments + 1;
    for i in 0..rings {
        for j in 0..segments {
            let (a, b) = (i * row + j, i * row + j + 1);
            let (c, d) = (b + row, a + row);
            p.indices.extend([d, c, b, d, b, a]);
        }
    }
    p
}

/// A `size` pixel square of `cells` by `cells` squares, white and `ink`.
#[must_use]
pub fn chequer(size: u32, cells: u32, ink: [u8; 4]) -> Image {
    let cell = (size / cells.max(1)).max(1);
    let mut rgba8 = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let odd = (x / cell + y / cell) % 2 == 1;
            rgba8.extend(if odd { ink } else { [255; 4] });
        }
    }
    Image {
        width: size,
        height: size,
        rgba8,
    }
}

/// A model of `parts`, each its own mesh and node placed at its offset in
/// metres.
#[must_use]
pub fn scene(
    parts: Vec<(Primitive, [f32; 3])>,
    materials: Vec<Material>,
    textures: Vec<Texture>,
) -> ModelScene {
    let mut s = ModelScene {
        materials,
        textures,
        bounds: Bounds {
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        },
        ..ModelScene::default()
    };
    for (k, (primitive, at)) in parts.into_iter().enumerate() {
        for q in &primitive.positions {
            for i in 0..3 {
                s.bounds.min[i] = s.bounds.min[i].min(q[i] + at[i]);
                s.bounds.max[i] = s.bounds.max[i].max(q[i] + at[i]);
            }
        }
        s.meshes.push(Mesh {
            name: format!("part {k}"),
            primitives: vec![primitive],
        });
        s.nodes.push(Node {
            name: format!("part {k}"),
            translation: at,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            mesh: Some(k),
            ..Node::default()
        });
        s.roots.push(k);
    }
    s
}
