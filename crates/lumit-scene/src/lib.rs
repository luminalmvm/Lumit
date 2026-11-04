//! A 3D model read from a file, as plain data.
//!
//! Every format Lumit opens (glTF and GLB, FBX, OBJ, and whatever Blender turns
//! into GLB for us) is read into one [`ModelScene`], so the draw never needs to
//! know where a model came from. It lives in the engine's cache, never in the
//! project: the project keeps the file and the layer's rows, and the scene is
//! read again from the file.
//!
//! The space is glTF's: metres, Y up, right-handed, a camera looking down -Z.
//! A composition is Y down with Z going away from the eye, so a model reaches
//! it turned half a turn about X, then scaled by its Model settings.

use std::path::{Path as FilePath, PathBuf};
use std::sync::atomic::AtomicBool;

use lumit_ingress::{Budget, Limits};

#[cfg(not(target_arch = "wasm32"))]
mod fbx_file;
mod files;
mod gltf_file;
pub mod pose;
mod settle;

/// Models built in code, for other crates' tests and the bench.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixtures;

pub mod blender;

/// One model: its tree of nodes and everything they point at.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelScene {
    pub nodes: Vec<Node>,
    /// The nodes with no parent, in file order.
    pub roots: Vec<usize>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub cameras: Vec<Camera>,
    pub lights: Vec<Light>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    /// The box round every mesh at rest, in model space.
    pub bounds: Bounds,
    /// Files the model asked for that could not be found, as it named them.
    pub missing: Vec<String>,
}

impl ModelScene {
    /// Roughly the memory the scene holds: its vertices, indices and decoded
    /// pictures, which is all but a sliver of it.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        let primitives: usize = self
            .meshes
            .iter()
            .flat_map(|m| &m.primitives)
            .map(|p| {
                let targets: usize = p
                    .morph_targets
                    .iter()
                    .map(|t| (t.positions.len() + t.normals.len() + t.tangents.len()) * 12)
                    .sum();
                (p.positions.len() + p.normals.len()) * 12
                    + (p.tangents.len() + p.colours.len() + p.weights.len()) * 16
                    + (p.uv0.len() + p.uv1.len() + p.joints.len()) * 8
                    + p.indices.len() * 4
                    + targets
            })
            .sum();
        let pictures: usize = self
            .textures
            .iter()
            .filter_map(|t| t.image.as_ref())
            .map(|i| i.rgba8.len())
            .sum();
        primitives + pictures
    }

    /// Node `k`'s place in model space: its own transform after all its
    /// parents'.
    #[must_use]
    pub fn node_world(&self, mut k: usize) -> glam::Mat4 {
        let mut world = glam::Mat4::IDENTITY;
        // Bounded, so a file whose parents loop can't hang the draw.
        for _ in 0..=self.nodes.len() {
            let Some(n) = self.nodes.get(k) else {
                break;
            };
            let turn = glam::Quat::from_array(n.rotation);
            let turn = if turn.length_squared() > 0.0 {
                turn.normalize()
            } else {
                glam::Quat::IDENTITY
            };
            world = glam::Mat4::from_scale_rotation_translation(
                n.scale.into(),
                turn,
                n.translation.into(),
            ) * world;
            match n.parent {
                Some(p) => k = p,
                None => break,
            }
        }
        world
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub translation: [f32; 3],
    /// A unit quaternion, x y z w.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub mesh: Option<usize>,
    pub camera: Option<usize>,
    pub light: Option<usize>,
    pub skin: Option<usize>,
    /// Morph target weights at rest, one per target of its mesh.
    pub weights: Vec<f32>,
    /// Where it is from its root by name, which a layer's rows find it by.
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
    /// The box round its vertices at rest, in its own space.
    pub bounds: Bounds,
}

/// Triangles that share one material. Every attribute list is either empty or
/// as long as `positions`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// xyz and the handedness in w, as glTF keeps them.
    pub tangents: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    pub uv1: Vec<[f32; 2]>,
    /// Linear RGBA.
    pub colours: Vec<[f32; 4]>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub morph_targets: Vec<MorphTarget>,
    /// Three to a triangle.
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

/// What one morph target adds to each vertex at full weight.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphTarget {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    /// The box round what it adds to each vertex.
    pub reach: Bounds,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

/// glTF's metallic-roughness material, with the extensions Lumit draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    /// Linear RGBA.
    pub base_colour: [f32; 4],
    pub base_colour_texture: Option<TextureRef>,
    pub metallic: f32,
    pub roughness: f32,
    /// Roughness in green, metallic in blue.
    pub metallic_roughness_texture: Option<TextureRef>,
    pub normal_texture: Option<TextureRef>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<TextureRef>,
    pub occlusion_strength: f32,
    /// Linear RGB.
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    pub emissive_texture: Option<TextureRef>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub unlit: bool,
    pub transmission: f32,
    pub ior: f32,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_colour: [1.0; 4],
            base_colour_texture: None,
            metallic: 1.0,
            roughness: 1.0,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            emissive_texture: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            unlit: false,
            transmission: 0.0,
            ior: 1.5,
            clearcoat: 0.0,
            clearcoat_roughness: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureRef {
    pub texture: usize,
    pub uv_set: u32,
    /// Offset, rotation in radians and scale (KHR_texture_transform).
    pub transform: Option<UvTransform>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvTransform {
    pub offset: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Wrap {
    #[default]
    Repeat,
    Mirror,
    Clamp,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Texture {
    pub name: String,
    /// Where the picture came from: a file beside the model, or none when it
    /// was packed inside it.
    pub file: Option<PathBuf>,
    /// The decoded picture, or `None` when it could not be read.
    pub image: Option<Image>,
    pub wrap_u: Wrap,
    pub wrap_v: Wrap,
    /// Nearest rather than linear filtering.
    pub nearest: bool,
}

/// Eight-bit RGBA, rows top first. Colour pictures are sRGB-encoded; the draw
/// decides by the slot it is used in, as glTF says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    /// Vertical field of view in radians.
    Perspective { yfov: f32 },
    /// Half the view's height, in metres.
    Orthographic { ymag: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub name: String,
    pub projection: Projection,
    pub znear: f32,
    pub zfar: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightKind {
    Point,
    Spot,
    Directional,
}

/// KHR_lights_punctual. Points and spots shine in candela, directional lights
/// in lux.
#[derive(Debug, Clone, PartialEq)]
pub struct Light {
    pub name: String,
    pub kind: LightKind,
    /// Linear RGB.
    pub colour: [f32; 3],
    pub intensity: f32,
    pub range: Option<f32>,
    /// Spot cone half-angles in radians.
    pub inner_cone: f32,
    pub outer_cone: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skin {
    pub joints: Vec<usize>,
    /// Column-major, one per joint.
    pub inverse_bind: Vec<[[f32; 4]; 4]>,
    /// The box round the vertices each joint moves, as they are at rest.
    pub boxes: Vec<Bounds>,
}

/// One embedded animation, a take.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clip {
    pub name: String,
    pub channels: Vec<Channel>,
    /// Seconds, the last key of any channel.
    pub duration: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    Translation,
    Rotation,
    Scale,
    Weights,
    /// One morph target's weight alone, one number a key.
    Weight(u16),
    /// A camera's vertical field of view in radians, one number a key.
    YFov,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
    /// Each key holds an in-tangent, the value and an out-tangent.
    CubicSpline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub path: Path,
    pub interpolation: Interpolation,
    /// Seconds, rising.
    pub times: Vec<f32>,
    /// Flat: three or four numbers a key (or a key's three parts), or one per
    /// morph target for Weights.
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("can't read {path}: {reason}")]
    Read { path: PathBuf, reason: String },
    #[error("{path} isn't a model Lumit can open")]
    Unsupported { path: PathBuf },
    #[error("{path} opens through Blender, and none was found")]
    NoBlender { path: PathBuf },
    #[error("{path} is larger than the memory a model may use")]
    TooLarge { path: PathBuf },
    #[error("cancelled")]
    Cancelled,
}

/// The extensions [`read`] opens itself, lower case. The rest of
/// [`is_model`]'s go through Blender.
pub const EXTENSIONS: [&str; 4] = ["gltf", "glb", "fbx", "obj"];

/// Whether the file at `path` is a 3D model [`read`] opens, by its name.
pub fn is_model(path: impl AsRef<FilePath>) -> bool {
    let ext = extension(path.as_ref());
    EXTENSIONS.contains(&ext.as_str()) || blender::EXTENSIONS.contains(&ext.as_str())
}

/// Read the model at `path`, held to `limits` and stopped when `cancel` is
/// set. A file only Blender reads is turned into a GLB in `cache` first, and
/// so is an FBX the native reader refuses. Never panics, whatever the file
/// holds.
pub fn read(
    path: &FilePath,
    limits: Limits,
    cache: &FilePath,
    cancel: &AtomicBool,
) -> Result<ModelScene, SceneError> {
    let ext = extension(path);
    if blender::EXTENSIONS.contains(&ext.as_str()) {
        return read_native(&blender::to_glb(path, cache, cancel)?, limits, cancel);
    }
    match read_native(path, limits, cancel) {
        Err(refused @ SceneError::Read { .. }) if ext == "fbx" => {
            match blender::to_glb(path, cache, cancel) {
                Ok(glb) => read_native(&glb, limits, cancel),
                Err(SceneError::Cancelled) => Err(SceneError::Cancelled),
                Err(_) => Err(refused),
            }
        }
        read => read,
    }
}

fn read_native(
    path: &FilePath,
    limits: Limits,
    cancel: &AtomicBool,
) -> Result<ModelScene, SceneError> {
    let mut r = files::Reader::new(path, Budget::new(limits), cancel);
    // Both parsers are other people's code, and the gltf crate's panic fixes
    // aren't released yet, so a panic in either is a file that won't open.
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut scene = match extension(path).as_str() {
            "gltf" | "glb" => gltf_file::read(&mut r),
            #[cfg(not(target_arch = "wasm32"))]
            "fbx" | "obj" => fbx_file::read(&mut r),
            _ => Err(SceneError::Unsupported {
                path: path.to_path_buf(),
            }),
        }?;
        link(&mut scene).map_err(|reason| r.fail(reason))?;
        tangents(&mut scene);
        scene.bounds = bounds(&scene);
        let broken = settle::settle(&mut scene);
        r.missing.extend(broken);
        Ok(scene)
    }));
    let mut scene = read.unwrap_or_else(|_| Err(r.fail("the file is damaged")))?;
    scene.missing = std::mem::take(&mut r.missing);
    // A picture two materials share is named once.
    scene.missing.sort();
    scene.missing.dedup();
    Ok(scene)
}

/// The other files a model pulls in (its .bin, pictures and .mtl) that are
/// there to be found, for packing and collecting. Never the model itself.
pub fn dependants(path: &FilePath) -> Vec<PathBuf> {
    let cancel = AtomicBool::new(false);
    let mut r = files::Reader::new(path, Budget::new(Limits::MODEL), &cancel);
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match extension(path).as_str() {
            "gltf" | "glb" => gltf_file::dependants(&mut r),
            #[cfg(not(target_arch = "wasm32"))]
            "fbx" | "obj" => fbx_file::dependants(&mut r),
            _ => Vec::new(),
        }
    }));
    let mut found = found.unwrap_or_default();
    found.sort();
    found.dedup();
    found
}

fn extension(path: &FilePath) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Parents from children, the roots, and a refusal for a tree that loops.
fn link(scene: &mut ModelScene) -> Result<(), &'static str> {
    let count = scene.nodes.len();
    for n in 0..count {
        for c in scene.nodes[n].children.clone() {
            let child = scene.nodes.get_mut(c).ok_or("a node's child isn't there")?;
            child.parent = Some(n);
        }
    }
    for start in 0..count {
        let mut at = scene.nodes[start].parent;
        for _ in 0..=count {
            match at {
                Some(p) => at = scene.nodes.get(p).and_then(|n| n.parent),
                None => break,
            }
        }
        if at.is_some() {
            return Err("its tree of parts loops");
        }
    }
    scene.roots = (0..count)
        .filter(|&n| scene.nodes[n].parent.is_none())
        .collect();
    Ok(())
}

/// Tangents for every primitive drawn with a normal map that came without
/// them, worked out the way the normal map's own baker did.
fn tangents(scene: &mut ModelScene) {
    struct Geometry<'a> {
        p: &'a Primitive,
        uv: &'a [[f32; 2]],
        out: Vec<[f32; 4]>,
    }
    impl Geometry<'_> {
        fn at(&self, face: usize, vert: usize) -> usize {
            self.p
                .indices
                .get(face * 3 + vert)
                .map_or(0, |&i| i as usize)
        }
    }
    impl bevy_mikktspace::Geometry for Geometry<'_> {
        fn num_faces(&self) -> usize {
            self.p.indices.len() / 3
        }
        fn num_vertices_of_face(&self, _: usize) -> usize {
            3
        }
        fn position(&self, face: usize, vert: usize) -> [f32; 3] {
            self.p
                .positions
                .get(self.at(face, vert))
                .copied()
                .unwrap_or_default()
        }
        fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
            self.p
                .normals
                .get(self.at(face, vert))
                .copied()
                .unwrap_or_default()
        }
        fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
            self.uv
                .get(self.at(face, vert))
                .copied()
                .unwrap_or_default()
        }
        fn set_tangent(
            &mut self,
            t: Option<bevy_mikktspace::TangentSpace>,
            face: usize,
            vert: usize,
        ) {
            // ponytail: a vertex shared by faces that disagree keeps the last
            // face's tangent. Splitting those vertices is the exact answer.
            let i = self.at(face, vert);
            if let (Some(t), Some(slot)) = (t, self.out.get_mut(i)) {
                *slot = t.tangent_encoded();
            }
        }
    }
    let materials = &scene.materials;
    for p in scene
        .meshes
        .iter_mut()
        .flat_map(|m| m.primitives.iter_mut())
    {
        let Some(normal_map) = p
            .material
            .and_then(|m| materials.get(m))
            .and_then(|m| m.normal_texture)
        else {
            continue;
        };
        let uv = if normal_map.uv_set == 1 {
            &p.uv1
        } else {
            &p.uv0
        };
        if !p.tangents.is_empty() || p.normals.is_empty() || uv.is_empty() {
            continue;
        }
        let mut g = Geometry {
            p,
            uv,
            out: vec![[1.0, 0.0, 0.0, 1.0]; p.positions.len()],
        };
        if bevy_mikktspace::generate_tangents(&mut g).is_ok() {
            p.tangents = g.out;
        }
    }
}

/// The box round every mesh in its place at rest.
fn bounds(scene: &ModelScene) -> Bounds {
    use glam::{Mat4, Quat, Vec3};
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let mut stack: Vec<(usize, Mat4, usize)> = scene
        .roots
        .iter()
        .map(|&n| (n, Mat4::IDENTITY, 0))
        .collect();
    while let Some((n, parent, depth)) = stack.pop() {
        // A loop in the tree would never end, so depth past the node count stops it.
        let Some(node) = scene.nodes.get(n).filter(|_| depth <= scene.nodes.len()) else {
            continue;
        };
        let world = parent
            * Mat4::from_scale_rotation_translation(
                Vec3::from(node.scale),
                Quat::from_array(node.rotation),
                Vec3::from(node.translation),
            );
        let points = node
            .mesh
            .and_then(|m| scene.meshes.get(m))
            .into_iter()
            .flat_map(|m| &m.primitives)
            .flat_map(|p| &p.positions);
        for &p in points {
            let p = world.transform_point3(Vec3::from(p));
            lo = lo.min(p);
            hi = hi.max(p);
        }
        stack.extend(node.children.iter().map(|&c| (c, world, depth + 1)));
    }
    if lo.x > hi.x {
        return Bounds::default();
    }
    Bounds {
        min: lo.to_array(),
        max: hi.to_array(),
    }
}
