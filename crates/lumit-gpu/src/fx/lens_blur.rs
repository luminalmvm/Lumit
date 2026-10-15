//! Depth of field pro's passes (`fx_lens_blur.wgsl`). The lens is traced on the
//! CPU, in lumit-core, into a table of kernels; this side keeps the last few
//! tables on the card and runs the frame.

use std::sync::{Arc, Mutex, OnceLock};

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One frame's settings, already resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensBlurOp {
    /// Names the bake; equal keys must mean equal tables.
    pub bake_key: u64,
    pub depth_bound: bool,
    /// The depth texture holds metres rather than a 0..1 pass.
    pub depth_metres: bool,
    /// Draw the metres used instead of the blur.
    pub display_depth: bool,
    pub depth_channel: u32,
    pub depth_invert: bool,
    pub focus_m: f32,
    pub use_focus_point: bool,
    /// Raster px.
    pub focus_point: [f32; 2],
    pub near_m: f32,
    pub far_m: f32,
    pub sensor_w_mm: f32,
    pub size: f32,
    pub blades: u32,
    pub rotation_deg: f32,
    pub threshold: f32,
    pub boost: f32,
    /// 0..1.
    pub mix: f32,
    /// Half-width cap, px.
    pub max_span_px: f32,
}

/// The traced table, as `lumit_core::fx::lens_blur::BokehBake` has it.
pub struct LensBlurBakeData {
    pub kernels: Vec<[f32; 3]>,
    pub span_mm: Vec<f32>,
    pub dz_max_mm: f32,
    pub field_max_mm: f32,
    pub focal_mm: f32,
    pub field_bins: u32,
    pub defocus_bins: u32,
    pub azimuth_bins: u32,
    pub kernel_res: u32,
}

/// The `P` uniform, word for word: sixteen u32 then sixteen f32.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    w: u32,
    h: u32,
    tiles_w: u32,
    tiles_h: u32,
    field_bins: u32,
    defocus_bins: u32,
    az_bins: u32,
    kernel_res: u32,
    atlas_cols: u32,
    depth_bound: u32,
    depth_channel: u32,
    depth_invert: u32,
    use_focus_point: u32,
    blades: u32,
    depth_metres: u32,
    display: u32,
    focal_mm: f32,
    dz_max_mm: f32,
    field_max_mm: f32,
    px_per_mm: f32,
    focus_m: f32,
    near_m: f32,
    far_m: f32,
    size: f32,
    focus_x: f32,
    focus_y: f32,
    rotation: f32,
    threshold: f32,
    boost: f32,
    mix_amt: f32,
    max_span: f32,
    _p2: f32,
}

const TILE: u32 = 16;
const ATLAS_COLS: u32 = 32;
/// Tables kept on the card. One is about 12 MB.
const CACHE_CAP: usize = 4;

struct GpuBake {
    atlas: wgpu::Texture,
    spans: wgpu::Buffer,
    dz_max_mm: f32,
    field_max_mm: f32,
    focal_mm: f32,
    field_bins: u32,
    defocus_bins: u32,
    azimuth_bins: u32,
    kernel_res: u32,
}

pub(super) struct LensBlurFx {
    prep: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    tiles: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    dilate: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    gather: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    sprite: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    finish: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    cache: Mutex<Vec<(u64, Arc<GpuBake>)>>,
}

/// One binding of a lens pass's layout.
#[derive(Clone, Copy)]
pub(super) enum Slot {
    Tex,
    Store(wgpu::TextureFormat),
    Uniform,
    Spans,
}

impl LensBlurFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-lens-blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_lens_blur.wgsl").into()),
        });
        let layout = |label: &str, slots: &[(u32, Slot)], vis: wgpu::ShaderStages| {
            bind_layout(device, label, slots, vis)
        };
        let compute = |label: &str, entry: &str, slots: &[(u32, Slot)]| {
            let bgl = layout(label, slots, wgpu::ShaderStages::COMPUTE);
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });
            let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
            (bgl, pipe)
        };
        use wgpu::TextureFormat as F;
        let prep = compute(
            "fx-lens-blur-prep",
            "lb_prep",
            &[
                (0, Slot::Tex),
                (1, Slot::Tex),
                (2, Slot::Store(F::Rgba32Float)),
                (3, Slot::Uniform),
                (11, Slot::Spans),
            ],
        );
        let tiles = compute(
            "fx-lens-blur-tiles",
            "lb_tiles",
            &[
                (4, Slot::Tex),
                (5, Slot::Store(F::R32Float)),
                (3, Slot::Uniform),
            ],
        );
        let dilate = compute(
            "fx-lens-blur-dilate",
            "lb_dilate",
            &[
                (6, Slot::Tex),
                (5, Slot::Store(F::R32Float)),
                (3, Slot::Uniform),
            ],
        );
        let gather = compute(
            "fx-lens-blur-gather",
            "lb_gather",
            &[
                (0, Slot::Tex),
                (1, Slot::Tex),
                (4, Slot::Tex),
                (6, Slot::Tex),
                (7, Slot::Tex),
                (8, Slot::Store(F::Rgba16Float)),
                (3, Slot::Uniform),
            ],
        );
        let finish = compute(
            "fx-lens-blur-finish",
            "lb_finish",
            &[
                (0, Slot::Tex),
                (4, Slot::Tex),
                (9, Slot::Tex),
                (10, Slot::Store(ctx.working())),
                (3, Slot::Uniform),
            ],
        );
        let sprite_bgl = layout(
            "fx-lens-blur-sprite",
            &[
                (0, Slot::Tex),
                (4, Slot::Tex),
                (7, Slot::Tex),
                (3, Slot::Uniform),
            ],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let sprite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-lens-blur-sprite"),
            bind_group_layouts: &[&sprite_bgl],
            push_constant_ranges: &[],
        });
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let sprite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fx-lens-blur-sprite"),
            layout: Some(&sprite_pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("lb_sprite_vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("lb_sprite_fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: F::Rgba16Float,
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
        Self {
            prep,
            tiles,
            dilate,
            gather,
            sprite: (sprite_bgl, sprite),
            finish,
            cache: Mutex::new(Vec::new()),
        }
    }

    /// The table for `key`, uploading a fresh bake on a miss.
    fn baked(
        &self,
        ctx: &GpuContext,
        key: u64,
        bake: &dyn Fn() -> Option<LensBlurBakeData>,
    ) -> Option<Arc<GpuBake>> {
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
                let hit = cache.remove(i);
                let out = hit.1.clone();
                cache.push(hit);
                return Some(out);
            }
        }
        let data = bake()?;
        let slices = data.span_mm.len() as u32;
        let k = data.kernel_res;
        if slices == 0 || k == 0 || data.kernels.len() != (slices * k * k) as usize {
            return None;
        }
        let rows = slices.div_ceil(ATLAS_COLS);
        let (aw, ah) = (ATLAS_COLS * k, rows * k);
        let mut level = vec![[0.0_f32; 3]; (aw * ah) as usize];
        for s in 0..slices {
            let (col, row) = (s % ATLAS_COLS, s / ATLAS_COLS);
            for y in 0..k {
                for x in 0..k {
                    level[((row * k + y) * aw + col * k + x) as usize] =
                        data.kernels[((s * k + y) * k + x) as usize];
                }
            }
        }
        // Smaller copies, each cell the sum of four, for small sprites to read
        // without falling between cells. Halved while a slice still has
        // whole cells.
        let mut texels = Vec::new();
        let (mut lw, mut lh, mut lk, mut mips) = (aw, ah, k, 1);
        loop {
            for c in &level {
                texels.extend(c.map(|v| half::f16::from_f32(v).to_bits()));
                texels.push(0);
            }
            if lk % 2 != 0 || mips == 4 {
                break;
            }
            let (nw, nh) = (lw / 2, lh / 2);
            level = (0..nw * nh)
                .map(|i| {
                    let (x, y) = (i % nw * 2, i / nw * 2);
                    let mut c = [0.0; 3];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let t = level[((y + dy) * lw + x + dx) as usize];
                        for ch in 0..3 {
                            c[ch] += t[ch];
                        }
                    }
                    c
                })
                .collect();
            (lw, lh, lk, mips) = (nw, nh, lk / 2, mips + 1);
        }
        use wgpu::util::DeviceExt;
        let atlas = ctx.device.create_texture_with_data(
            &ctx.queue,
            &wgpu::TextureDescriptor {
                label: Some("fx-lens-blur-atlas"),
                size: wgpu::Extent3d {
                    width: aw,
                    height: ah,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&texels),
        );
        let spans = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-blur-spans"),
                contents: bytemuck::cast_slice(&data.span_mm),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let out = Arc::new(GpuBake {
            atlas,
            spans,
            dz_max_mm: data.dz_max_mm,
            field_max_mm: data.field_max_mm,
            focal_mm: data.focal_mm,
            field_bins: data.field_bins,
            defocus_bins: data.defocus_bins,
            azimuth_bins: data.azimuth_bins,
            kernel_res: k,
        });
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= CACHE_CAP {
                cache.remove(0);
            }
            cache.push((key, out.clone()));
        }
        Some(out)
    }
}

/// A bind group layout from `(binding, kind)` pairs.
pub(super) fn bind_layout(
    device: &wgpu::Device,
    label: &str,
    slots: &[(u32, Slot)],
    vis: wgpu::ShaderStages,
) -> wgpu::BindGroupLayout {
    let entries: Vec<_> = slots
        .iter()
        .map(|&(binding, slot)| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: match slot {
                Slot::Tex => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                Slot::Store(format) => wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                Slot::Uniform => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                Slot::Spans => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            },
            count: None,
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

/// A frame-sized scratch texture of a fixed format, charged to this frame.
pub(super) fn scratch(
    ctx: &GpuContext,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    label: &str,
) -> wgpu::Texture {
    ctx.charge_vram(crate::texture_bytes(format, w, h));
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

impl FxEngine {
    /// Depth of field pro over a linear working texture (premultiplied), returning a
    /// new one of the same size. `depth` is the matte, or `None` to put the whole
    /// frame at the far distance. `bake` runs only when no table for
    /// `op.bake_key` is kept; a bake that fails leaves the picture as it was.
    #[allow(clippy::too_many_arguments)]
    pub fn lens_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        depth: Option<&wgpu::Texture>,
        op: &LensBlurOp,
        bake: &dyn Fn() -> Option<LensBlurBakeData>,
    ) -> wgpu::Texture {
        let fx = self.lens_blur.get_or_init(|| LensBlurFx::new(ctx));
        let Some(b) = fx.baked(ctx, op.bake_key, bake) else {
            return src.clone();
        };
        use wgpu::util::DeviceExt;
        use wgpu::TextureUsages as U;
        let (tw, th) = (w.div_ceil(TILE), h.div_ceil(TILE));
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lens-blur-params"),
                contents: bytemuck::bytes_of(&Params {
                    w,
                    h,
                    tiles_w: tw,
                    tiles_h: th,
                    field_bins: b.field_bins,
                    defocus_bins: b.defocus_bins,
                    az_bins: b.azimuth_bins,
                    kernel_res: b.kernel_res,
                    atlas_cols: ATLAS_COLS,
                    depth_bound: u32::from(op.depth_bound && depth.is_some()),
                    depth_channel: op.depth_channel,
                    depth_invert: u32::from(op.depth_invert),
                    use_focus_point: u32::from(op.use_focus_point),
                    blades: op.blades.max(3),
                    depth_metres: u32::from(op.depth_metres),
                    display: u32::from(op.display_depth),
                    focal_mm: b.focal_mm,
                    dz_max_mm: b.dz_max_mm,
                    field_max_mm: b.field_max_mm,
                    px_per_mm: w as f32 / op.sensor_w_mm.max(1.0),
                    focus_m: op.focus_m,
                    near_m: op.near_m,
                    far_m: op.far_m,
                    size: op.size,
                    focus_x: op.focus_point[0],
                    focus_y: op.focus_point[1],
                    rotation: op.rotation_deg.to_radians(),
                    threshold: op.threshold,
                    boost: op.boost,
                    mix_amt: op.mix,
                    max_span: op.max_span_px,
                    _p2: 0.0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let info = scratch(
            ctx,
            w,
            h,
            wgpu::TextureFormat::Rgba32Float,
            U::STORAGE_BINDING | U::TEXTURE_BINDING,
            "fx-lens-blur-info",
        );
        let tiles = scratch(
            ctx,
            tw,
            th,
            wgpu::TextureFormat::R32Float,
            U::STORAGE_BINDING | U::TEXTURE_BINDING,
            "fx-lens-blur-tiles",
        );
        let reach = scratch(
            ctx,
            tw,
            th,
            wgpu::TextureFormat::R32Float,
            U::STORAGE_BINDING | U::TEXTURE_BINDING,
            "fx-lens-blur-reach",
        );
        let acc = scratch(
            ctx,
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            U::STORAGE_BINDING | U::TEXTURE_BINDING | U::RENDER_ATTACHMENT,
            "fx-lens-blur-acc",
        );
        let out = work_texture(ctx, w, h, "fx-lens-blur-out");

        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (v_src, v_info, v_tiles, v_reach, v_acc, v_atlas, v_out) = (
            view(src),
            view(&info),
            view(&tiles),
            view(&reach),
            view(&acc),
            view(&b.atlas),
            view(&out),
        );
        // No matte: the source stands in a slot the kernel never reads.
        let v_depth = view(depth.unwrap_or(src));
        let bind = |bgl: &wgpu::BindGroupLayout, entries: &[(u32, wgpu::BindingResource<'_>)]| {
            let entries: Vec<_> = entries
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: resource.clone(),
                })
                .collect();
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-lens-blur"),
                layout: bgl,
                entries: &entries,
            })
        };
        use wgpu::BindingResource::TextureView as T;
        let u = ubuf.as_entire_binding();
        let prep = bind(
            &fx.prep.0,
            &[
                (0, T(&v_src)),
                (1, T(&v_depth)),
                (2, T(&v_info)),
                (3, u.clone()),
                (11, b.spans.as_entire_binding()),
            ],
        );
        let tile = bind(
            &fx.tiles.0,
            &[(4, T(&v_info)), (5, T(&v_tiles)), (3, u.clone())],
        );
        let dilate = bind(
            &fx.dilate.0,
            &[(6, T(&v_tiles)), (5, T(&v_reach)), (3, u.clone())],
        );
        let gather = bind(
            &fx.gather.0,
            &[
                (0, T(&v_src)),
                (1, T(&v_depth)),
                (4, T(&v_info)),
                (6, T(&v_reach)),
                (7, T(&v_atlas)),
                (8, T(&v_acc)),
                (3, u.clone()),
            ],
        );
        let sprite = bind(
            &fx.sprite.0,
            &[
                (0, T(&v_src)),
                (4, T(&v_info)),
                (7, T(&v_atlas)),
                (3, u.clone()),
            ],
        );
        let finish = bind(
            &fx.finish.0,
            &[
                (0, T(&v_src)),
                (4, T(&v_info)),
                (9, T(&v_acc)),
                (10, T(&v_out)),
                (3, u),
            ],
        );

        let mut enc = ctx.encoder("fx-lens-blur");
        let mut run = |pipe: &wgpu::ComputePipeline, group: &wgpu::BindGroup, gx: u32, gy: u32| {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-lens-blur"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(gx.div_ceil(8), gy.div_ceil(8), 1);
        };
        run(&fx.prep.1, &prep, w, h);
        // The Depth view only needs what prep worked out.
        if !op.display_depth {
            run(&fx.tiles.1, &tile, tw, th);
            run(&fx.dilate.1, &dilate, tw, th);
            run(&fx.gather.1, &gather, w, h);
        }
        if !op.display_depth {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx-lens-blur-sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &v_acc,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rp.set_pipeline(&fx.sprite.1);
            rp.set_bind_group(0, &sprite, &[]);
            rp.draw(0..6, 0..w * h);
        }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-lens-blur-finish"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&fx.finish.1);
        pass.set_bind_group(0, &finish, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        drop(pass);
        drop(enc);
        out
    }
}

/// Built on first use: most projects never draw a Depth of field pro.
pub(super) type LazyLensBlur = OnceLock<LensBlurFx>;
