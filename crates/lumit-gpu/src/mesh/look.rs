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
}

/// What every material's bind group shares: the white picture an empty slot
/// reads.
pub(super) struct Shared {
    white: wgpu::TextureView,
}

impl Shared {
    pub(super) fn new(white: &wgpu::Texture) -> Self {
        Self {
            white: white.create_view(&Default::default()),
        }
    }
}

/// What a material's bind group is made of besides its numbers: each slot's
/// uv transform and picture, as the upload made them.
pub(super) struct Kept {
    pub(super) uv: [[f32; 4]; 10],
    pub(super) bound: Vec<wgpu::TextureView>,
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
                match (m.unlit, m.soft_turns) {
                    (true, _) => 1.0,
                    (false, true) => -1.0,
                    (false, false) => 0.0,
                },
                f32::from(u8::from(pictures[2].is_some())),
            ],
            extra: [m.transmission, m.ior, m.clearcoat, m.clearcoat_roughness],
            slots: uv,
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
        for look in looks {
            let (Some(kept), Some(slot)) = (model.kept.get(look.index), out.get_mut(look.index))
            else {
                continue;
            };
            let pictures = (look.pictures.each_ref())
                .map(|p| p.as_ref().map(|t| view(&self.levelled(ctx, t))));
            let m = &look.material;
            let pictures = (pictures.each_ref().map(Option::as_ref), look.textures);
            let group = kept.group(ctx, self, m, pictures, behind.as_ref());
            slot.0 = group;
            (slot.1, slot.2) = (m.alpha_mode == AlphaMode::Blend, m.double_sided);
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
