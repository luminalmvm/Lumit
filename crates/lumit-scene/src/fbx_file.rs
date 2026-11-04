//! FBX and OBJ (with its MTL), read with `ufbx`.

use std::sync::atomic::Ordering;

use ufbx::{
    AllocatorOpts, BakeOpts, CoordinateAxes, ErrorType, GeometryTransformHandling, LoadOpts,
    MaterialMap, ProgressCb, ProgressResult,
};

use crate::files::{find, obj_libraries, Reader, VERTEX_BYTES};
use crate::{
    AlphaMode, Camera, Channel, Clip, Image, Interpolation, Light, LightKind, Material, Mesh,
    ModelScene, Node, Path as Target, Primitive, Projection, SceneError, Texture, TextureRef, Wrap,
};

pub(crate) fn read(r: &mut Reader) -> Result<ModelScene, SceneError> {
    let root = load(r, false)?;
    let mut scene = ModelScene::default();

    r.budget
        .take_items(root.nodes.len() as u64)
        .map_err(|_| r.too_large())?;
    // ufbx numbers each kind of element from nought in its own list, so a
    // typed id is the element's place in ours.
    let id = |e: &ufbx::Element| e.typed_id as usize;
    for node in &root.nodes {
        let t = &node.local_transform;
        scene.nodes.push(Node {
            name: node.element.name.to_string(),
            parent: node.parent.as_ref().map(|p| id(&p.element)),
            children: node.children.iter().map(|c| id(&c.element)).collect(),
            translation: v3(t.translation),
            rotation: [
                t.rotation.x as f32,
                t.rotation.y as f32,
                t.rotation.z as f32,
                t.rotation.w as f32,
            ],
            scale: v3(t.scale),
            mesh: node.mesh.as_ref().map(|m| id(&m.element)),
            camera: node.camera.as_ref().map(|c| id(&c.element)),
            light: node.light.as_ref().map(|l| id(&l.element)),
            // ponytail: no skins or blend shapes from FBX yet; the animation
            // item reads them.
            skin: None,
            weights: Vec::new(),
        });
    }

    for mesh in &root.meshes {
        let mut corners = vec![0u32; mesh.max_face_triangles * 3];
        let mut primitives = Vec::new();
        for part in &mesh.material_parts {
            if part.num_triangles == 0 {
                continue;
            }
            r.take(part.num_triangles * 3 * VERTEX_BYTES)?;
            let second_uv = mesh.uv_sets.get(1).map(|s| &s.vertex_uv);
            let mut p = Primitive::default();
            for &face in &part.face_indices {
                let Some(&face) = mesh.faces.get(face as usize) else {
                    continue;
                };
                let made = ufbx::triangulate_face(&mut corners, mesh, face) as usize;
                for &i in corners.iter().take(made * 3) {
                    let i = i as usize;
                    p.positions.push(v3(mesh.vertex_position[i]));
                    if mesh.vertex_normal.exists {
                        p.normals.push(v3(mesh.vertex_normal[i]));
                    }
                    // FBX and OBJ put the first row of a picture at v = 1.
                    if mesh.vertex_uv.exists {
                        let uv = mesh.vertex_uv[i];
                        p.uv0.push([uv.x as f32, 1.0 - uv.y as f32]);
                    }
                    if let Some(uv) = second_uv.filter(|s| s.exists).map(|s| s[i]) {
                        p.uv1.push([uv.x as f32, 1.0 - uv.y as f32]);
                    }
                    if mesh.vertex_color.exists {
                        let c = mesh.vertex_color[i];
                        p.colours
                            .push([c.x as f32, c.y as f32, c.z as f32, c.w as f32]);
                    }
                }
            }
            // ponytail: every corner is its own vertex, three times the
            // vertices a welded mesh has. ufbx::generate_indices welds them.
            p.indices = (0..p.positions.len() as u32).collect();
            p.material = mesh
                .materials
                .get(part.index as usize)
                .map(|m| id(&m.element));
            primitives.push(p);
        }
        scene.meshes.push(Mesh {
            name: mesh.element.name.to_string(),
            primitives,
        });
    }

    scene.materials = root.materials.iter().map(|m| material(m, &id)).collect();

    for t in &root.textures {
        r.check()?;
        let (file, bytes) = if t.content.is_empty() {
            let file = [&t.absolute_filename, &t.relative_filename, &t.filename]
                .into_iter()
                .filter(|n| !n.is_empty())
                .find_map(|n| find(&r.dir, n));
            match file {
                Some(f) => {
                    let bytes = r.read_file(&f)?;
                    (Some(f), Some(bytes))
                }
                None => {
                    let named = [&t.relative_filename, &t.filename]
                        .into_iter()
                        .find(|n| !n.is_empty());
                    r.missing.extend(named.map(|n| n.to_string()));
                    (None, None)
                }
            }
        } else {
            (None, Some(t.content.to_vec()))
        };
        let name = [&t.relative_filename, &t.filename, &t.element.name]
            .into_iter()
            .find(|n| !n.is_empty())
            .map(|n| n.to_string())
            .unwrap_or_default();
        let image = match bytes {
            Some(bytes) => r.decode(&bytes, &name)?,
            None => None,
        };
        let wrap = |w: ufbx::WrapMode| match w {
            ufbx::WrapMode::Repeat => Wrap::Repeat,
            ufbx::WrapMode::Clamp => Wrap::Clamp,
        };
        scene.textures.push(Texture {
            name: t.element.name.to_string(),
            file,
            image,
            wrap_u: wrap(t.wrap_u),
            wrap_v: wrap(t.wrap_v),
            nearest: false,
        });
    }
    pack_metal_rough(r, &root, &mut scene, &id)?;

    scene.cameras = root
        .cameras
        .iter()
        .map(|c| Camera {
            name: c.element.name.to_string(),
            projection: match c.projection_mode {
                ufbx::ProjectionMode::Perspective => Projection::Perspective {
                    yfov: (c.field_of_view_deg.y as f32).to_radians(),
                },
                ufbx::ProjectionMode::Orthographic => Projection::Orthographic {
                    ymag: c.orthographic_size.y as f32 / 2.0,
                },
            },
            znear: c.near_plane as f32,
            zfar: Some(c.far_plane as f32),
        })
        .collect();

    scene.lights = root
        .lights
        .iter()
        .map(|l| Light {
            name: l.element.name.to_string(),
            kind: match l.type_ {
                ufbx::LightType::Directional => LightKind::Directional,
                ufbx::LightType::Spot => LightKind::Spot,
                // Area and volume lights have no glTF kind; a point is closest.
                _ => LightKind::Point,
            },
            colour: [l.color.x as f32, l.color.y as f32, l.color.z as f32],
            intensity: l.intensity as f32,
            range: None,
            // ufbx gives whole cone angles in degrees.
            inner_cone: (l.inner_angle as f32 / 2.0).to_radians(),
            outer_cone: (l.outer_angle as f32 / 2.0).to_radians(),
        })
        .collect();

    for stack in &root.anim_stacks {
        r.check()?;
        let limit = memory_limit(r);
        let opts = BakeOpts {
            temp_allocator: AllocatorOpts {
                memory_limit: limit,
                ..AllocatorOpts::default()
            },
            result_allocator: AllocatorOpts {
                memory_limit: limit,
                ..AllocatorOpts::default()
            },
            trim_start_time: true,
            ..BakeOpts::default()
        };
        let baked = ufbx::bake_anim(&root, &stack.anim, opts).map_err(|e| r.fail(e.description))?;
        let mut clip = Clip {
            name: stack.element.name.to_string(),
            duration: baked.playback_duration as f32,
            channels: Vec::new(),
        };
        for n in &baked.nodes {
            let node = n.typed_id as usize;
            let mut push = |path, times: Vec<f32>, values: Vec<f32>| {
                clip.channels.push(Channel {
                    node,
                    path,
                    interpolation: Interpolation::Linear,
                    times,
                    values,
                });
            };
            if !n.constant_translation {
                let (t, v) = vec3_keys(&n.translation_keys);
                push(Target::Translation, t, v);
            }
            if !n.constant_rotation {
                let times = n.rotation_keys.iter().map(|k| k.time as f32).collect();
                let values = n
                    .rotation_keys
                    .iter()
                    .flat_map(|k| [k.value.x, k.value.y, k.value.z, k.value.w].map(|c| c as f32))
                    .collect();
                push(Target::Rotation, times, values);
            }
            if !n.constant_scale {
                let (t, v) = vec3_keys(&n.scale_keys);
                push(Target::Scale, t, v);
            }
        }
        r.take(clip.channels.iter().map(|c| c.values.len() * 8).sum())?;
        scene.clips.push(clip);
    }
    Ok(scene)
}

/// The texture files an FBX or OBJ names that exist, and the OBJ's MTL files.
pub(crate) fn dependants(r: &mut Reader) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    if is_obj(r) {
        if let Ok(obj) = r.read_file(r.path) {
            found.extend(obj_libraries(&obj).iter().filter_map(|n| find(&r.dir, n)));
        }
    }
    if let Ok(root) = load(r, true) {
        for t in &root.textures {
            let file = [&t.absolute_filename, &t.relative_filename, &t.filename]
                .into_iter()
                .filter(|n| !n.is_empty())
                .find_map(|n| find(&r.dir, n));
            found.extend(file);
        }
    }
    found
}

fn is_obj(r: &Reader) -> bool {
    r.path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("obj"))
}

fn memory_limit(r: &Reader) -> usize {
    usize::try_from(r.budget.bytes_remaining()).unwrap_or(usize::MAX)
}

/// The file loaded by ufbx into glTF's space, Y up and in metres, held to the
/// budget and stopped by the cancel flag. `light` skips geometry and
/// animation, for when only the names of other files are wanted.
fn load(r: &mut Reader, light: bool) -> Result<ufbx::SceneRoot, SceneError> {
    // OBJ names its MTL by a path that may be from another machine, so it's
    // found here and handed over, rather than ufbx opening files itself.
    let mut mtl = Vec::new();
    if is_obj(r) {
        let obj = r.read_file(r.path)?;
        for name in obj_libraries(&obj) {
            match find(&r.dir, &name) {
                Some(file) => mtl.extend(r.read_file(&file)?),
                None => r.missing.push(name),
            }
            mtl.push(b'\n');
        }
    }
    let path = r
        .path
        .to_str()
        .ok_or_else(|| r.fail("its path isn't valid text"))?;
    let cancel = r.cancel;
    let mut progress = |_: &ufbx::Progress| {
        if cancel.load(Ordering::Relaxed) {
            ProgressResult::Cancel
        } else {
            ProgressResult::Continue
        }
    };
    let limit = memory_limit(r);
    let opts = LoadOpts {
        temp_allocator: AllocatorOpts {
            memory_limit: limit,
            ..AllocatorOpts::default()
        },
        result_allocator: AllocatorOpts {
            memory_limit: limit,
            ..AllocatorOpts::default()
        },
        ignore_geometry: light,
        ignore_animation: light,
        target_axes: CoordinateAxes::right_handed_y_up(),
        target_camera_axes: CoordinateAxes::right_handed_y_up(),
        target_light_axes: CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        generate_missing_normals: true,
        geometry_transform_handling: GeometryTransformHandling::HelperNodes,
        obj_mtl_data: if mtl.is_empty() {
            ufbx::BlobOpt::default()
        } else {
            mtl.as_slice().into()
        },
        progress_cb: ProgressCb::Mut(&mut progress),
        ..LoadOpts::default()
    };
    ufbx::load_file(path, opts).map_err(|e| match e.type_ {
        ErrorType::Cancelled => SceneError::Cancelled,
        ErrorType::MemoryLimit | ErrorType::AllocationLimit | ErrorType::OutOfMemory => {
            r.too_large()
        }
        ErrorType::UnrecognizedFileFormat => SceneError::Unsupported {
            path: r.path.to_path_buf(),
        },
        _ => r.fail(e.description),
    })
}

fn material(m: &ufbx::Material, id: &dyn Fn(&ufbx::Element) -> usize) -> Material {
    let p = &m.pbr;
    let real = |map: &MaterialMap, default: f32| {
        if map.has_value {
            map.value_vec4.x as f32
        } else {
            default
        }
    };
    let tex = |map: &MaterialMap| texture_of(map, id);
    // A picture named with no colour (an OBJ's map_Kd with no Kd) shows as it
    // is, rather than multiplied down to black.
    let base = match p.base_color.has_value || p.base_color.texture.is_none() {
        true => p.base_color.value_vec4,
        false => ufbx::Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        },
    };
    let factor = real(&p.base_factor, 1.0);
    let opacity = real(&p.opacity, 1.0);
    let emit = p.emission_color.value_vec4;
    let defaults = Material::default();
    Material {
        name: m.element.name.to_string(),
        base_colour: [
            base.x as f32 * factor,
            base.y as f32 * factor,
            base.z as f32 * factor,
            opacity,
        ],
        base_colour_texture: tex(&p.base_color),
        metallic: real(&p.metalness, 0.0),
        roughness: real(&p.roughness, 0.5),
        // Filled in by pack_metal_rough once the pictures are read.
        metallic_roughness_texture: None,
        normal_texture: tex(&p.normal_map),
        normal_scale: 1.0,
        occlusion_texture: tex(&p.ambient_occlusion),
        occlusion_strength: 1.0,
        emissive: [emit.x as f32, emit.y as f32, emit.z as f32],
        emissive_strength: real(&p.emission_factor, 1.0),
        emissive_texture: tex(&p.emission_color),
        alpha_mode: if opacity < 1.0 || tex(&p.opacity).is_some() {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        alpha_cutoff: defaults.alpha_cutoff,
        double_sided: m.features.double_sided.enabled,
        soft_turns: false,
        unlit: m.features.unlit.enabled,
        transmission: real(&p.transmission_factor, 0.0),
        ior: real(&p.specular_ior, defaults.ior),
        clearcoat: real(&p.coat_factor, 0.0),
        clearcoat_roughness: real(&p.coat_roughness, 0.0),
    }
}

/// The picture a material map reads, if it has one.
fn texture_of(map: &MaterialMap, id: &dyn Fn(&ufbx::Element) -> usize) -> Option<TextureRef> {
    let t = map.texture.as_ref().filter(|_| map.texture_enabled)?;
    // A layered texture points at its file textures; the first stands in.
    let t = if t.has_file {
        t
    } else {
        t.file_textures.iter().next()?
    };
    Some(TextureRef {
        texture: id(&t.element),
        uv_set: 0,
        transform: None,
    })
}

/// FBX keeps metalness and roughness as two pictures. Each material with
/// either gets one picture of both, roughness in green and metalness in blue
/// as glTF packs them, at the size of the first, and its factor for that
/// picture set to one.
fn pack_metal_rough(
    r: &mut Reader,
    root: &ufbx::Scene,
    scene: &mut ModelScene,
    id: &dyn Fn(&ufbx::Element) -> usize,
) -> Result<(), SceneError> {
    let mut packed = Vec::new();
    for (m, out) in root.materials.iter().zip(scene.materials.iter_mut()) {
        let [metal, rough] = [&m.pbr.metalness, &m.pbr.roughness].map(|map| {
            let t = texture_of(map, id)?;
            scene.textures.get(t.texture)?.image.as_ref()
        });
        let Some((w, h)) = metal
            .or(rough)
            .map(|i| (i.width as usize, i.height as usize))
        else {
            continue;
        };
        r.take(w * h * 4)?;
        // Each picture's red at the same place, white where there's none.
        let at = |i: Option<&Image>, x: usize, y: usize| {
            i.map_or(255, |i| {
                let (iw, ih) = (i.width as usize, i.height as usize);
                i.rgba8[(y * ih / h * iw + x * iw / w) * 4]
            })
        };
        let rgba8 = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [0, at(rough, x, y), at(metal, x, y), 255]))
            .collect();
        packed.push(Texture {
            name: format!("{} metal-rough", out.name),
            image: Some(Image {
                width: w as u32,
                height: h as u32,
                rgba8,
            }),
            ..Texture::default()
        });
        out.metallic_roughness_texture = Some(TextureRef {
            texture: scene.textures.len() + packed.len() - 1,
            uv_set: 0,
            transform: None,
        });
        if metal.is_some() {
            out.metallic = 1.0;
        }
        if rough.is_some() {
            out.roughness = 1.0;
        }
    }
    scene.textures.extend(packed);
    Ok(())
}

fn v3(v: ufbx::Vec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}

fn vec3_keys(keys: &[ufbx::BakedVec3]) -> (Vec<f32>, Vec<f32>) {
    (
        keys.iter().map(|k| k.time as f32).collect(),
        keys.iter().flat_map(|k| v3(k.value)).collect(),
    )
}
