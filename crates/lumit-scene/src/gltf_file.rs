//! glTF and GLB, read with the `gltf` crate.

use gltf::animation::util::ReadOutputs;
use gltf::mesh::Mode;
use gltf::texture::{MagFilter, WrappingMode};

use crate::files::{Reader, VERTEX_BYTES};
use crate::{
    AlphaMode, Camera, Channel, Clip, Interpolation, Light, LightKind, Material, Mesh, ModelScene,
    MorphTarget, Node, Path as Target, Primitive, Projection, SceneError, Skin, Texture,
    TextureRef, UvTransform, Wrap,
};

pub(crate) fn read(r: &mut Reader) -> Result<ModelScene, SceneError> {
    let bytes = r.read_file(r.path)?;
    let gltf::Gltf { document, mut blob } =
        gltf::Gltf::from_slice(&bytes).map_err(|e| r.fail(e))?;
    drop(bytes);

    let mut buffers = Vec::new();
    for buffer in document.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => blob.take(),
            gltf::buffer::Source::Uri(uri) => r.fetch(uri)?,
        };
        let Some(data) = data else {
            return Err(r.fail("its geometry file is missing"));
        };
        if data.len() < buffer.length() {
            return Err(r.fail("a buffer is shorter than the file says"));
        }
        buffers.push(data);
    }
    let data = |b: gltf::Buffer| buffers.get(b.index()).map(Vec::as_slice);

    let mut scene = ModelScene::default();
    for mesh in document.meshes() {
        let mut primitives = Vec::new();
        for p in mesh.primitives() {
            let count = p.get(&gltf::Semantic::Positions).map_or(0, |a| a.count());
            let index_count = p.indices().map_or(count, |a| a.count());
            r.take(count.saturating_mul(VERTEX_BYTES) + index_count.saturating_mul(4))?;
            let reader = p.reader(data);
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let n = positions.len();
            let raw: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..n as u32).collect(),
            };
            let Some(indices) = triangles(p.mode(), &raw) else {
                continue;
            };
            if indices.iter().any(|&i| i as usize >= n) {
                return Err(r.fail("a mesh points past its own vertices"));
            }
            let morph_targets = reader
                .read_morph_targets()
                .map(|(pos, nor, tan)| MorphTarget {
                    positions: fit(pos.map(Iterator::collect).unwrap_or_default(), n),
                    normals: fit(nor.map(Iterator::collect).unwrap_or_default(), n),
                    tangents: fit(tan.map(Iterator::collect).unwrap_or_default(), n),
                })
                .collect();
            primitives.push(Primitive {
                normals: fit(
                    reader
                        .read_normals()
                        .map(Iterator::collect)
                        .unwrap_or_default(),
                    n,
                ),
                tangents: fit(
                    reader
                        .read_tangents()
                        .map(Iterator::collect)
                        .unwrap_or_default(),
                    n,
                ),
                uv0: fit(
                    reader
                        .read_tex_coords(0)
                        .map(|t| t.into_f32().collect())
                        .unwrap_or_default(),
                    n,
                ),
                uv1: fit(
                    reader
                        .read_tex_coords(1)
                        .map(|t| t.into_f32().collect())
                        .unwrap_or_default(),
                    n,
                ),
                colours: fit(
                    reader
                        .read_colors(0)
                        .map(|c| c.into_rgba_f32().collect())
                        .unwrap_or_default(),
                    n,
                ),
                joints: fit(
                    reader
                        .read_joints(0)
                        .map(|j| j.into_u16().collect())
                        .unwrap_or_default(),
                    n,
                ),
                weights: fit(
                    reader
                        .read_weights(0)
                        .map(|w| w.into_f32().collect())
                        .unwrap_or_default(),
                    n,
                ),
                morph_targets,
                positions,
                indices,
                material: p.material().index(),
            });
        }
        scene.meshes.push(Mesh {
            name: mesh.name().unwrap_or_default().to_string(),
            primitives,
        });
    }

    scene.materials = document.materials().map(material).collect();

    for t in document.textures() {
        r.check()?;
        let (file, bytes) = match t.source().source() {
            gltf::image::Source::View { view, .. } => {
                let start = view.offset();
                let bytes = buffers
                    .get(view.buffer().index())
                    .and_then(|b| b.get(start..start.saturating_add(view.length())))
                    .map(<[u8]>::to_vec);
                (None, bytes)
            }
            gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
                (None, r.fetch(uri)?)
            }
            gltf::image::Source::Uri { uri, .. } => {
                let file = crate::files::find(&r.dir, &crate::files::unescape(uri));
                (file, r.fetch(uri)?)
            }
        };
        let name = match t.source().source() {
            gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => uri,
            _ => t.name().or(t.source().name()).unwrap_or_default(),
        };
        let image = match bytes {
            Some(bytes) => r.decode(&bytes, name)?,
            None => None,
        };
        let sampler = t.sampler();
        scene.textures.push(Texture {
            name: t
                .name()
                .or(t.source().name())
                .unwrap_or_default()
                .to_string(),
            file,
            image,
            wrap_u: wrap(sampler.wrap_s()),
            wrap_v: wrap(sampler.wrap_t()),
            nearest: sampler.mag_filter() == Some(MagFilter::Nearest),
        });
    }

    r.budget
        .take_items(document.nodes().len() as u64)
        .map_err(|_| r.too_large())?;
    for node in document.nodes() {
        let (translation, rotation, scale) = node.transform().decomposed();
        scene.nodes.push(Node {
            name: node.name().unwrap_or_default().to_string(),
            parent: None,
            children: node.children().map(|c| c.index()).collect(),
            translation,
            rotation,
            scale,
            mesh: node.mesh().map(|m| m.index()),
            camera: node.camera().map(|c| c.index()),
            light: node.light().map(|l| l.index()),
            skin: node.skin().map(|s| s.index()),
            weights: node
                .weights()
                .or_else(|| node.mesh().and_then(|m| m.weights()))
                .map(<[f32]>::to_vec)
                .unwrap_or_default(),
        });
    }

    scene.cameras = document
        .cameras()
        .map(|c| match c.projection() {
            gltf::camera::Projection::Perspective(p) => Camera {
                name: c.name().unwrap_or_default().to_string(),
                projection: Projection::Perspective { yfov: p.yfov() },
                znear: p.znear(),
                zfar: p.zfar(),
            },
            gltf::camera::Projection::Orthographic(o) => Camera {
                name: c.name().unwrap_or_default().to_string(),
                projection: Projection::Orthographic { ymag: o.ymag() },
                znear: o.znear(),
                zfar: Some(o.zfar()),
            },
        })
        .collect();

    scene.lights = document
        .lights()
        .into_iter()
        .flatten()
        .map(|l| {
            use gltf::khr_lights_punctual::Kind;
            let (kind, inner_cone, outer_cone) = match l.kind() {
                Kind::Directional => (LightKind::Directional, 0.0, 0.0),
                Kind::Point => (LightKind::Point, 0.0, 0.0),
                Kind::Spot {
                    inner_cone_angle,
                    outer_cone_angle,
                } => (LightKind::Spot, inner_cone_angle, outer_cone_angle),
            };
            Light {
                name: l.name().unwrap_or_default().to_string(),
                kind,
                colour: l.color(),
                intensity: l.intensity(),
                range: l.range(),
                inner_cone,
                outer_cone,
            }
        })
        .collect();

    for skin in document.skins() {
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut inverse_bind: Vec<_> = skin
            .reader(data)
            .read_inverse_bind_matrices()
            .map(Iterator::collect)
            .unwrap_or_default();
        inverse_bind.resize(joints.len(), identity);
        scene.skins.push(Skin {
            joints,
            inverse_bind,
        });
    }

    for animation in document.animations() {
        r.check()?;
        let mut clip = Clip {
            name: animation.name().unwrap_or_default().to_string(),
            ..Clip::default()
        };
        for channel in animation.channels() {
            let reader = channel.reader(data);
            let (Some(times), Some(outputs)) = (reader.read_inputs(), reader.read_outputs()) else {
                continue;
            };
            let times: Vec<f32> = times.collect();
            let (path, values): (Target, Vec<f32>) = match outputs {
                ReadOutputs::Translations(v) => (Target::Translation, v.flatten().collect()),
                ReadOutputs::Rotations(v) => (Target::Rotation, v.into_f32().flatten().collect()),
                ReadOutputs::Scales(v) => (Target::Scale, v.flatten().collect()),
                ReadOutputs::MorphTargetWeights(v) => (Target::Weights, v.into_f32().collect()),
            };
            r.take((times.len() + values.len()) * 4)?;
            clip.duration = clip.duration.max(times.last().copied().unwrap_or(0.0));
            clip.channels.push(Channel {
                node: channel.target().node().index(),
                path,
                interpolation: match channel.sampler().interpolation() {
                    gltf::animation::Interpolation::Step => Interpolation::Step,
                    gltf::animation::Interpolation::Linear => Interpolation::Linear,
                    gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
                },
                times,
                values,
            });
        }
        scene.clips.push(clip);
    }
    Ok(scene)
}

/// A primitive's indices as a plain triangle list, or `None` for points and
/// lines, which a model draws nothing for.
fn triangles(mode: Mode, raw: &[u32]) -> Option<Vec<u32>> {
    match mode {
        Mode::Triangles => Some(raw[..raw.len() / 3 * 3].to_vec()),
        Mode::TriangleStrip => Some(
            raw.windows(3)
                .enumerate()
                .flat_map(|(i, w)| {
                    // Every other triangle of a strip winds the other way.
                    if i % 2 == 0 {
                        [w[0], w[1], w[2]]
                    } else {
                        [w[1], w[0], w[2]]
                    }
                })
                .collect(),
        ),
        Mode::TriangleFan => {
            let (&first, rest) = raw.split_first()?;
            Some(rest.windows(2).flat_map(|w| [first, w[0], w[1]]).collect())
        }
        Mode::Points | Mode::Lines | Mode::LineLoop | Mode::LineStrip => None,
    }
}

/// An attribute as long as the positions, or none at all.
fn fit<T>(v: Vec<T>, n: usize) -> Vec<T> {
    if v.len() == n {
        v
    } else {
        Vec::new()
    }
}

fn wrap(mode: WrappingMode) -> Wrap {
    match mode {
        WrappingMode::Repeat => Wrap::Repeat,
        WrappingMode::MirroredRepeat => Wrap::Mirror,
        WrappingMode::ClampToEdge => Wrap::Clamp,
    }
}

fn material(m: gltf::Material) -> Material {
    let pbr = m.pbr_metallic_roughness();
    let ext = |name: &str, field: &str| {
        m.extension_value(name)
            .and_then(|v| v.get(field))
            .and_then(serde_json::Value::as_f64)
            .map(|v| v as f32)
    };
    let defaults = Material::default();
    Material {
        name: m.name().unwrap_or_default().to_string(),
        base_colour: pbr.base_color_factor(),
        base_colour_texture: pbr.base_color_texture().map(|t| info(&t)),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        metallic_roughness_texture: pbr.metallic_roughness_texture().map(|t| info(&t)),
        normal_texture: m.normal_texture().map(|t| TextureRef {
            texture: t.texture().index(),
            uv_set: t.tex_coord(),
            transform: transform_json(t.extension_value("KHR_texture_transform")),
        }),
        normal_scale: m.normal_texture().map_or(1.0, |t| t.scale()),
        occlusion_texture: m.occlusion_texture().map(|t| TextureRef {
            texture: t.texture().index(),
            uv_set: t.tex_coord(),
            transform: transform_json(t.extension_value("KHR_texture_transform")),
        }),
        occlusion_strength: m.occlusion_texture().map_or(1.0, |t| t.strength()),
        emissive: m.emissive_factor(),
        emissive_strength: m.emissive_strength().unwrap_or(1.0),
        emissive_texture: m.emissive_texture().map(|t| info(&t)),
        alpha_mode: match m.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        },
        alpha_cutoff: m.alpha_cutoff().unwrap_or(defaults.alpha_cutoff),
        double_sided: m.double_sided(),
        soft_turns: false,
        unlit: m.unlit(),
        transmission: m.transmission().map_or(0.0, |t| t.transmission_factor()),
        ior: m.ior().unwrap_or(defaults.ior),
        // The gltf crate doesn't know clearcoat, so it's read from the JSON.
        clearcoat: ext("KHR_materials_clearcoat", "clearcoatFactor").unwrap_or(0.0),
        clearcoat_roughness: ext("KHR_materials_clearcoat", "clearcoatRoughnessFactor")
            .unwrap_or(0.0),
    }
}

fn info(t: &gltf::texture::Info) -> TextureRef {
    let transform = t.texture_transform();
    TextureRef {
        texture: t.texture().index(),
        uv_set: transform
            .as_ref()
            .and_then(|x| x.tex_coord())
            .unwrap_or(t.tex_coord()),
        transform: transform.map(|x| UvTransform {
            offset: x.offset(),
            rotation: x.rotation(),
            scale: x.scale(),
        }),
    }
}

/// KHR_texture_transform on the normal and occlusion slots, which the gltf
/// crate only hands over as JSON.
fn transform_json(value: Option<&serde_json::Value>) -> Option<UvTransform> {
    let value = value?;
    let pair = |key: &str, default: [f32; 2]| {
        value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32]))
            .unwrap_or(default)
    };
    Some(UvTransform {
        offset: pair("offset", [0.0, 0.0]),
        rotation: value
            .get("rotation")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32,
        scale: pair("scale", [1.0, 1.0]),
    })
}

/// The files a glTF names by path that are there to be found.
pub(crate) fn dependants(r: &mut Reader) -> Vec<std::path::PathBuf> {
    let Ok(bytes) = r.read_file(r.path) else {
        return Vec::new();
    };
    let Ok(gltf) = gltf::Gltf::from_slice_without_validation(&bytes) else {
        return Vec::new();
    };
    let buffers = gltf.document.buffers().filter_map(|b| match b.source() {
        gltf::buffer::Source::Uri(uri) => Some(uri),
        gltf::buffer::Source::Bin => None,
    });
    let images = gltf.document.images().filter_map(|i| match i.source() {
        gltf::image::Source::Uri { uri, .. } => Some(uri),
        gltf::image::Source::View { .. } => None,
    });
    buffers
        .chain(images)
        .filter(|uri| !uri.starts_with("data:"))
        .filter_map(|uri| crate::files::find(&r.dir, &crate::files::unescape(uri)))
        .collect()
}
