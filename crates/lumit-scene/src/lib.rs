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

use std::path::PathBuf;

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
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
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
    #[error("{path} is larger than the memory a model may use")]
    TooLarge { path: PathBuf },
    #[error("cancelled")]
    Cancelled,
}
