//! A material drawn as its Model layer's rows have it rather than as its file
//! does: other numbers, a layer's picture in a slot, and glass that bends
//! what is behind the model.

use super::{GpuModel, MeshFx};
use crate::GpuContext;
use lumit_scene::{AlphaMode, Material};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniform {
    base_colour: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    mode: [f32; 4],
    extra: [f32; 4],
    slots: [[f32; 4]; 10],
    projector: [[f32; 4]; 4],
    plate: [[f32; 4]; 3],
    projection: [f32; 4],
    projector_eye: [f32; 4],
}

/// One material as its layer's rows have it this frame.
pub struct Look {
    /// Which of the file's materials it is.
    pub index: usize,
    pub material: Material,
    /// A layer's picture for the base colour, the emission, the opacity and
    /// the normals, each read on the model's own uvs.
    pub pictures: [Option<wgpu::Texture>; 4],
    /// Whether the file's base colour and metal-rough pictures still show.
    pub textures: bool,
    /// The camera its base colour is projected from, in place of the model's
    /// uvs. Read only when it has a base colour picture.
    pub projector: Option<super::projector::Projector>,
}

/// What every material's bind group shares: the white picture an empty slot
/// reads, and the depth one on its uvs binds.
pub(super) struct Shared {
    white: wgpu::TextureView,
    no_depth: wgpu::TextureView,
}

impl Shared {
    pub(super) fn new(device: &wgpu::Device, white: &wgpu::Texture) -> Self {
        Self {
            white: white.create_view(&Default::default()),
            no_depth: super::shadow::no_maps(device)
                .0
                .create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    ..Default::default()
                }),
        }
    }
}

/// What a material's bind group is made of besides its numbers: each slot's
/// uv transform and picture, as the upload made them.
pub(super) struct Kept {
    pub(super) uv: [[f32; 4]; 10],
    pub(super) bound: Vec<wgpu::TextureView>,
    pub(super) readings: [super::samplers::Reading; 5],
}

impl Kept {
    /// `m`'s bind group, with `pictures` in place of the file's base colour,
    /// emission and normals and as its opacity, the file's base colour and
    /// metal-rough pictures left out without `textures`, and `behind` for
    /// glass to bend.
    pub(super) fn group(
        &self,
        ctx: &GpuContext,
        fx: &MeshFx,
        m: &Material,
        (pictures, textures): ([Option<&wgpu::TextureView>; 4], bool),
        behind: Option<&wgpu::TextureView>,
        projector: Option<&super::projector::Projector>,
    ) -> wgpu::BindGroup {
        let shared = &fx.shared;
        let mut uv = self.uv;
        if !textures {
            uv[1][3] = 0.0;
            uv[3][3] = 0.0;
        }
        // A layer's normals were read as a picture, so the shader puts back
        // the encoding a normal map is stored in.
        if pictures[3].is_some() {
            uv[5][3] = 2.0;
        }
        let e = m.emissive.map(|c| c * m.emissive_strength);
        let projector = projector.filter(|_| pictures[0].is_some());
        let (from, plate, projection, projector_eye) =
            super::projector::Projector::uniform(projector);
        let uniform = MaterialUniform {
            base_colour: m.base_colour,
            emissive: [e[0], e[1], e[2], 0.0],
            params: [
                m.metallic,
                m.roughness,
                m.normal_scale,
                m.occlusion_strength,
            ],
            mode: [
                match m.alpha_mode {
                    AlphaMode::Opaque => 0.0,
                    AlphaMode::Mask => 1.0,
                    AlphaMode::Blend => 2.0,
                },
                m.alpha_cutoff,
                f32::from(u8::from(m.unlit)),
                f32::from(u8::from(pictures[2].is_some())),
            ],
            extra: [m.transmission, m.ior, m.clearcoat, m.clearcoat_roughness],
            slots: uv,
            projector: from,
            plate,
            projection,
            projector_eye,
        };
        let buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-material"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }];
        for (k, view) in (0u32..).zip(&self.bound) {
            let view = match k {
                0 => pictures[0].unwrap_or(if textures { view } else { &shared.white }),
                1 if !textures => &shared.white,
                2 => pictures[3].unwrap_or(view),
                4 => pictures[1].unwrap_or(view),
                _ => view,
            };
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + k,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        for (binding, view) in [(11, pictures[2]), (12, behind)] {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view.unwrap_or(&shared.white)),
            });
        }
        let depth = projector.and_then(|p| p.depth.as_ref()).map(|d| {
            d.texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                ..Default::default()
            })
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 14,
            resource: wgpu::BindingResource::TextureView(
                depth.as_ref().unwrap_or(&shared.no_depth),
            ),
        });
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-material"),
            layout: &fx.material_layout,
            entries: &entries,
        })
    }
}

impl MeshFx {
    /// Each of `model`'s materials as `looks` have them this frame: whether
    /// it blends and whether it's double-sided beside its bind group. The
    /// upload's own where nothing has changed.
    pub(super) fn looked(
        &self,
        ctx: &GpuContext,
        model: &GpuModel,
        looks: &[Look],
        behind: Option<&wgpu::Texture>,
    ) -> Vec<(wgpu::BindGroup, bool, bool, wgpu::BindGroup)> {
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let behind = behind.map(view);
        let mut out = model.materials.clone();
        // Materials sharing a picture share its levels too.
        let mut levels: Vec<(wgpu::Texture, wgpu::Texture)> = Vec::new();
        for look in looks {
            let (Some(kept), Some(slot)) = (model.kept.get(look.index), out.get_mut(look.index))
            else {
                continue;
            };
            let pictures = (look.pictures.each_ref()).map(|p| {
                p.as_ref().map(|t| {
                    let known = levels.iter().find(|(from, _)| from == t);
                    let made = known.map(|(_, to)| to.clone()).unwrap_or_else(|| {
                        let to = self.levelled(ctx, t);
                        levels.push((t.clone(), to.clone()));
                        to
                    });
                    view(&made)
                })
            });
            let m = &look.material;
            let pictures = (pictures.each_ref().map(Option::as_ref), look.textures);
            let group = kept.group(
                ctx,
                self,
                m,
                pictures,
                behind.as_ref(),
                look.projector.as_ref(),
            );
            slot.0 = group;
            (slot.1, slot.2) = (m.alpha_mode == AlphaMode::Blend, m.double_sided);
            // A projected picture never wraps.
            if look.projector.is_some() && look.pictures[0].is_some() {
                let mut readings = kept.readings;
                readings[0] = super::samplers::PLATE;
                slot.3 = self.samplers.group(&ctx.device, readings);
            }
        }
        out
    }
}

impl MeshFx {
    /// `picture` with its smaller levels made, so a layer seen small on a
    /// model doesn't shimmer. As it is in any other format than the pass's.
    fn levelled(&self, ctx: &GpuContext, picture: &wgpu::Texture) -> wgpu::Texture {
        if picture.format() != super::COLOUR
            || !picture.usage().contains(wgpu::TextureUsages::COPY_SRC)
        {
            return picture.clone();
        }
        let size = picture.size();
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh-layer-picture"),
            size,
            mip_level_count: super::upload::level_count(size.width, size.height),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: super::COLOUR,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let mut encoder = ctx.encoder("mesh-layer-picture");
        encoder.copy_texture_to_texture(picture.as_image_copy(), texture.as_image_copy(), size);
        drop(encoder);
        self.mips(ctx, &texture);
        texture
    }
}
