//! Sprite flare's passes (`fx_sprite_flare.wgsl`): the lights made ready
//! (found in the picture, hidden behind a layer), every instance on every
//! light drawn as one instanced quad, added into a half-float picture, then
//! that picture added to the layer.

use std::sync::OnceLock;

use crate::GpuContext;

use super::lens_blur::{bind_layout, scratch, Slot};
use super::{work_texture, FxEngine};

/// Floats in one instance, as `lumit_core::fx::flare::instances` packs it.
/// This crate never depends on `lumit-core`, so the shape is restated.
pub const INSTANCE_FLOATS: usize = 36;

/// The light slots Bright spots fills: Lens flare's detection finds this
/// many at most.
pub const SPOT_SLOTS: usize = 16;

/// Lens flare's detection tile side, as its WGSL has it.
const DETECT_TILE: u32 = 16;

/// Bright spots: the lights are found in the layer's own picture.
#[derive(Debug, Clone, Copy)]
pub struct SpotsOp {
    pub threshold: f32,
    pub softness: f32,
    /// A spot's own hue tints its light.
    pub use_colour: bool,
    /// The Colour row, times every light.
    pub colour: [f32; 3],
    /// The most a spot's light is worth.
    pub gain: f32,
}

/// Hiding: each light is dimmed by how much of a disc round it this picture
/// covers.
#[derive(Debug, Clone, Copy)]
pub struct HideOp<'a> {
    pub picture: &'a wgpu::Texture,
    /// Luma rather than alpha.
    pub luma: bool,
    /// Raster pixels.
    pub radius: f32,
}

/// Hiding by depth: each light is dimmed by how much of a disc round it is
/// nearer than the light.
#[derive(Debug, Clone, Copy)]
pub struct DepthOp<'a> {
    /// Metres in red.
    pub picture: &'a wgpu::Texture,
    /// How far away the light is, metres.
    pub metres: f32,
    /// Raster pixels.
    pub radius: f32,
}

/// One frame's flare, already expanded.
#[derive(Debug, Clone, Copy)]
pub struct SpriteFlareOp<'a> {
    pub instances: &'a [[f32; INSTANCE_FLOATS]],
    /// Each light: x, y, gain and scale, then its colour and a 0. Ignored
    /// when `spots` finds them instead.
    pub lights: &'a [[f32; 8]],
    /// Raster pixels.
    pub centre: [f32; 2],
    /// Raster pixels; 0 is no fade.
    pub edge_fade: f32,
    pub spots: Option<SpotsOp>,
    pub hide: Option<HideOp<'a>>,
    pub depth: Option<DepthOp<'a>>,
    /// The Texture layer's picture, which an element's Layer texture reads.
    pub layer: Option<&'a wgpu::Texture>,
    /// Every texture an instance can name, one after another, made once:
    /// handed over on the first frame and kept.
    pub textures: &'a [f32],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    w: f32,
    h: f32,
    centre: [f32; 2],
    edge_fade: f32,
    lights: u32,
    has_layer: u32,
    _pad: f32,
}

/// What `sf_lights` is told.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightPass {
    count: u32,
    spots: u32,
    /// 0 no hiding, 1 alpha, 2 luma.
    hide: u32,
    use_colour: u32,
    radius: f32,
    gain: f32,
    w: f32,
    h: f32,
    colour: [f32; 4],
    depth: u32,
    light_m: f32,
    _pad: [u32; 2],
}

pub(super) struct SpriteFlareFx {
    draw: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    add: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    lights: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    /// Lens flare's own detection kernels, built from its shader.
    detect: (
        wgpu::BindGroupLayout,
        wgpu::ComputePipeline,
        wgpu::ComputePipeline,
    ),
    textures: OnceLock<wgpu::Buffer>,
}

/// What the quads are added into before they meet the layer: blendable on
/// every card, whatever the project's depth.
const LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl SpriteFlareFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-sprite-flare"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_sprite_flare.wgsl").into()),
        });
        let draw_bgl = bind_layout(
            device,
            "fx-sprite-flare-draw",
            &[
                (0, Slot::Uniform),
                (1, Slot::Spans),
                (2, Slot::Spans),
                (11, Slot::Spans),
                (12, Slot::Tex),
            ],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let layout = |bgl: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-sprite-flare"),
                bind_group_layouts: &[bgl],
                push_constant_ranges: &[],
            })
        };
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fx-sprite-flare-draw"),
            layout: Some(&layout(&draw_bgl)),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("sf_vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("sf_fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: LIGHT_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: add,
                        alpha: add,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            // Never multisampled: draw order alone fixes the sum, so a frame
            // adds up the same every time.
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let compute = |bgl: &wgpu::BindGroupLayout, module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout(bgl)),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let c = wgpu::ShaderStages::COMPUTE;
        let add_bgl = bind_layout(
            device,
            "fx-sprite-flare-add",
            &[
                (3, Slot::Tex),
                (4, Slot::Tex),
                (5, Slot::Store(ctx.working())),
            ],
            c,
        );
        let add_pipe = compute(&add_bgl, &module, "sf_add");
        let lights_bgl = bind_layout(
            device,
            "fx-sprite-flare-lights",
            &[
                (6, Slot::Uniform),
                (7, Slot::Spans),
                (8, Slot::Spans),
                (9, Slot::Tex),
                (10, Slot::StorageRw),
                (13, Slot::Tex),
            ],
            c,
        );
        let lights_pipe = compute(&lights_bgl, &module, "sf_lights");
        let detect_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-sprite-flare-detect"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_lens_flare_detect.wgsl").into()),
        });
        let detect_bgl = bind_layout(
            device,
            "fx-sprite-flare-detect",
            &[
                (0, Slot::Tex),
                (1, Slot::StorageRw),
                (2, Slot::StorageRw),
                (3, Slot::Uniform),
            ],
            c,
        );
        let tiles = compute(&detect_bgl, &detect_module, "detect_tiles");
        let pick = compute(&detect_bgl, &detect_module, "detect_pick");
        Self {
            draw: (draw_bgl, draw),
            add: (add_bgl, add_pipe),
            lights: (lights_bgl, lights_pipe),
            detect: (detect_bgl, tiles, pick),
            textures: OnceLock::new(),
        }
    }
}

/// Built on first use: most projects never draw a Sprite flare.
pub(super) type LazySpriteFlare = OnceLock<SpriteFlareFx>;

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

impl FxEngine {
    /// The flare in `op` added to `src`, alpha untouched. With nothing to
    /// draw it hands `src` back as it is.
    pub fn sprite_flare(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SpriteFlareOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        use wgpu::BindingResource as R;
        use wgpu::BufferUsages as B;
        use wgpu::TextureUsages as U;
        let slots = if op.spots.is_some() {
            SPOT_SLOTS
        } else {
            op.lights.len()
        };
        let Ok(count) = u32::try_from(op.instances.len() * slots) else {
            return src.clone();
        };
        if count == 0 || w == 0 || h == 0 {
            return src.clone();
        }
        let fx = self.sprite_flare.get_or_init(|| SpriteFlareFx::new(ctx));
        let buffer = |label, contents: &[u8], usage| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        // A storage binding may not be empty.
        let texels = fx.textures.get_or_init(|| {
            let data: &[f32] = if op.textures.is_empty() {
                &[0.0]
            } else {
                op.textures
            };
            buffer(
                "fx-sprite-flare-textures",
                bytemuck::cast_slice(data),
                B::STORAGE,
            )
        });
        let params = buffer(
            "fx-sprite-flare-params",
            bytemuck::bytes_of(&Params {
                w: w as f32,
                h: h as f32,
                centre: op.centre,
                edge_fade: op.edge_fade,
                lights: slots as u32,
                has_layer: u32::from(op.layer.is_some()),
                _pad: 0.0,
            }),
            B::UNIFORM,
        );
        let instances = buffer(
            "fx-sprite-flare-instances",
            bytemuck::cast_slice(op.instances),
            B::STORAGE,
        );
        let mut given = op.lights.to_vec();
        given.resize(slots.max(1), [0.0; 8]);
        let given = buffer(
            "fx-sprite-flare-given",
            bytemuck::cast_slice(&given),
            B::STORAGE,
        );
        // Detection writes its sixteen lights straight here.
        let found = buffer(
            "fx-sprite-flare-found",
            bytemuck::cast_slice(&[[0.0f32; 8]; SPOT_SLOTS]),
            B::STORAGE,
        );
        let lights = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-sprite-flare-lights"),
            size: (slots.max(1) * 32) as u64,
            usage: B::STORAGE,
            mapped_at_creation: false,
        });
        let spots = op.spots.unwrap_or(SpotsOp {
            threshold: 0.0,
            softness: 0.0,
            use_colour: false,
            colour: [1.0; 3],
            gain: 0.0,
        });
        let light_pass = buffer(
            "fx-sprite-flare-light-pass",
            bytemuck::bytes_of(&LightPass {
                count: slots as u32,
                spots: u32::from(op.spots.is_some()),
                hide: op.hide.map_or(0, |h| if h.luma { 2 } else { 1 }),
                use_colour: u32::from(spots.use_colour),
                radius: op
                    .hide
                    .map(|h| h.radius)
                    .or(op.depth.map(|d| d.radius))
                    .unwrap_or(0.0)
                    .max(0.0),
                gain: spots.gain,
                w: w as f32,
                h: h as f32,
                colour: [spots.colour[0], spots.colour[1], spots.colour[2], 0.0],
                depth: u32::from(op.depth.is_some()),
                light_m: op.depth.map_or(0.0, |d| d.metres),
                _pad: [0; 2],
            }),
            B::UNIFORM,
        );
        let light = scratch(
            ctx,
            w,
            h,
            LIGHT_FORMAT,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
            "fx-sprite-flare-light",
        );
        let out = work_texture(ctx, w, h, "fx-sprite-flare-out");
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (v_src, v_light, v_out) = (view(src), view(&light), view(&out));
        let v_hide = view(op.hide.map_or(src, |h| h.picture));
        let v_depth = view(op.depth.map_or(src, |d| d.picture));
        let v_layer = view(op.layer.unwrap_or(src));
        let group = |layout, entries: &[wgpu::BindGroupEntry<'_>]| {
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-sprite-flare"),
                layout,
                entries,
            })
        };
        let mut enc = ctx.encoder("fx-sprite-flare");
        let pass = |enc: &mut wgpu::CommandEncoder,
                    pipe: &wgpu::ComputePipeline,
                    bind: &wgpu::BindGroup,
                    n: [u32; 2]| {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-sprite-flare"),
                timestamp_writes: None,
            });
            p.set_pipeline(pipe);
            p.set_bind_group(0, bind, &[]);
            p.dispatch_workgroups(n[0], n[1], 1);
        };
        if let Some(s) = op.spots {
            // Lens flare's DetectParams: size, tiles, the gate, whether to
            // keep the colour, no Invert, and a white tint.
            let (tx, ty) = (w.div_ceil(DETECT_TILE), h.div_ceil(DETECT_TILE));
            let one = 1.0f32.to_bits();
            let detect = [
                w,
                h,
                tx,
                ty,
                s.threshold.to_bits(),
                s.softness.to_bits(),
                u32::from(s.use_colour),
                0,
                one,
                one,
                one,
                0,
            ];
            let detect = buffer(
                "fx-sprite-flare-detect",
                bytemuck::cast_slice(&detect),
                B::UNIFORM,
            );
            let tiles = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fx-sprite-flare-tiles"),
                // Twenty words a tile, as Lens flare's `Tile`.
                size: u64::from(tx * ty) * 80,
                usage: B::STORAGE,
                mapped_at_creation: false,
            });
            let bind = group(
                &fx.detect.0,
                &[
                    entry(0, R::TextureView(&v_src)),
                    entry(1, tiles.as_entire_binding()),
                    entry(2, found.as_entire_binding()),
                    entry(3, detect.as_entire_binding()),
                ],
            );
            pass(&mut enc, &fx.detect.1, &bind, [tx, ty]);
            pass(&mut enc, &fx.detect.2, &bind, [1, 1]);
        }
        let bind = group(
            &fx.lights.0,
            &[
                entry(6, light_pass.as_entire_binding()),
                entry(7, given.as_entire_binding()),
                entry(8, found.as_entire_binding()),
                entry(9, R::TextureView(&v_hide)),
                entry(10, lights.as_entire_binding()),
                entry(13, R::TextureView(&v_depth)),
            ],
        );
        pass(
            &mut enc,
            &fx.lights.1,
            &bind,
            [(slots as u32).div_ceil(64), 1],
        );
        let draw_group = group(
            &fx.draw.0,
            &[
                entry(0, params.as_entire_binding()),
                entry(1, instances.as_entire_binding()),
                entry(2, lights.as_entire_binding()),
                entry(11, texels.as_entire_binding()),
                entry(12, R::TextureView(&v_layer)),
            ],
        );
        let add_group = group(
            &fx.add.0,
            &[
                entry(3, R::TextureView(&v_src)),
                entry(4, R::TextureView(&v_light)),
                entry(5, R::TextureView(&v_out)),
            ],
        );
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fx-sprite-flare-draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &v_light,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        rp.set_pipeline(&fx.draw.1);
        rp.set_bind_group(0, &draw_group, &[]);
        rp.draw(0..6, 0..count);
        drop(rp);
        pass(
            &mut enc,
            &fx.add.1,
            &add_group,
            [w.div_ceil(8), h.div_ceil(8)],
        );
        drop(enc);
        out
    }
}
