//! Hologram's passes: halved copies of the layer and of the Backdrop layer,
//! then one dispatch of `fx_hologram.wgsl`, compiled after Holographic foil's
//! kernel, whose sheet, lights and surface it shares. Its kernel comes lean
//! (no wear, no Height or Environment map), which most frames need and which
//! compiles faster, or full; once one is drawn, the other is built on a
//! thread of its own.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

use crate::GpuContext;

use super::holographic_foil::HOLOGRAPHIC_FOIL_ROWS;
use super::lens_blur::{bind_layout, Slot};
use super::{work_texture, FxEngine};

/// The format of the halved copies, whatever the project's depth.
const MIP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// At most this many halvings for Hologram: a blur of a few hundred pixels.
const MIP_LEVELS: u32 = 8;

/// The row holding wear in y, as the kernel's R_RELIEF_MORE.
const ROW_WEAR: usize = 193;

/// The pictures only the full kernel reads: the Height and Environment maps,
/// as bits of the flags.
const FULL_PICTURES: u32 = 0b10_0010;

/// One frame of Hologram. The rows mirror
/// `lumit_core::fx::effects::hologram::HologramParams`.
#[derive(Debug, Clone, Copy)]
pub struct HologramOp<'a> {
    pub rows: [[f32; 4]; HOLOGRAPHIC_FOIL_ROWS],
    /// Scales the replay, prepared at the seam.
    pub matte: Option<&'a wgpu::Texture>,
    pub height_map: Option<&'a wgpu::Texture>,
    pub environment: Option<&'a wgpu::Texture>,
    pub depth_map: Option<&'a wgpu::Texture>,
    pub backdrop: Option<&'a wgpu::Texture>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    /// x: one bit for each picture bound, as Holographic foil's kernel reads
    /// them, then the depth map and the backdrop.
    flags: [u32; 4],
    rows: [[f32; 4]; HOLOGRAPHIC_FOIL_ROWS],
}

pub(super) struct HologramFx {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pl: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    /// Lean, then full.
    pipelines: [OnceLock<wgpu::ComputePipeline>; 2],
    halver: Halver,
}

/// Halved copies of a picture, each texel the mean of the four below it, for
/// reading it blurred by any amount. Hologram reads its layer and Backdrop
/// layer through them, Holographic foil its alpha for a bevel.
pub(super) struct Halver {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

fn compute(
    device: &wgpu::Device,
    label: &str,
    pl: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    constants: &HashMap<String, f64>,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(pl),
        module,
        entry_point: Some(entry),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants,
            ..Default::default()
        },
        cache: None,
    })
}

fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[layout],
        push_constant_ranges: &[],
    })
}

impl HologramFx {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-hologram"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(
                    ctx.working(),
                    concat!(
                        include_str!("../fx_holo_core.wgsl"),
                        include_str!("../fx_holo_relief.wgsl"),
                        include_str!("../fx_holo_shatter.wgsl"),
                        include_str!("../fx_holo_glitter.wgsl"),
                        include_str!("../fx_holo_sparkle.wgsl"),
                        include_str!("../fx_holographic_foil.wgsl"),
                        include_str!("../fx_hologram.wgsl")
                    ),
                )
                .into(),
            ),
        });
        // The foil's mask, direction and pitch maps are not read here.
        let mut slots = vec![
            (0, Slot::Tex),
            (1, Slot::Uniform),
            (2, Slot::Store(ctx.working())),
            (3, Slot::Tex),
            (4, Slot::Tex),
        ];
        slots.extend((8..13).map(|b| (b, Slot::Tex)));
        let layout = bind_layout(device, "fx-hologram", &slots, wgpu::ShaderStages::COMPUTE);
        let pl = pipeline_layout(device, "fx-hologram", &layout);
        Self {
            device: device.clone(),
            layout,
            pl,
            module,
            pipelines: Default::default(),
            halver: Halver::new(device),
        }
    }

    /// The lean (0) or full (1) kernel, built on first use with FULL fixed.
    /// A kernel another thread is building is waited for, never built twice.
    fn pipeline(&self, full: usize) -> &wgpu::ComputePipeline {
        let full = full.min(1);
        self.pipelines[full].get_or_init(|| {
            let constants = HashMap::from([("FULL".to_owned(), full as f64)]);
            compute(
                &self.device,
                "fx-hologram",
                &self.pl,
                &self.module,
                "hologram",
                &constants,
            )
        })
    }
}

impl Halver {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-hologram-mip"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_hologram_mip.wgsl").into()),
        });
        let layout = bind_layout(
            device,
            "fx-hologram-mip",
            &[(0, Slot::Tex), (1, Slot::Store(MIP_FORMAT))],
            wgpu::ShaderStages::COMPUTE,
        );
        let pipeline = compute(
            device,
            "fx-hologram-mip",
            &pipeline_layout(device, "fx-hologram-mip", &layout),
            &module,
            "hologram_mip",
            &HashMap::new(),
        );
        Self { layout, pipeline }
    }

    /// Halved copies of `tex` (`w` by `h`): level 0 is half its size, each
    /// next level half again, down to a few texels and at most eight
    /// halvings, or with `whole` down to one texel on the longer side, for a
    /// blur as wide as the picture.
    pub(super) fn mips(
        &self,
        ctx: &GpuContext,
        tex: &wgpu::Texture,
        w: u32,
        h: u32,
        whole: bool,
    ) -> wgpu::Texture {
        let (w0, h0) = ((w / 2).max(1), (h / 2).max(1));
        let (most, least) = if whole {
            (u32::MAX, 1)
        } else {
            (MIP_LEVELS, 4)
        };
        let mut levels = 1u32;
        while levels < most && (w0 >> levels).max(h0 >> levels) >= least {
            levels += 1;
        }
        let bytes: u64 = (0..levels)
            .map(|k| crate::texture_bytes(MIP_FORMAT, (w0 >> k).max(1), (h0 >> k).max(1)))
            .sum();
        ctx.charge_vram(bytes);
        let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx-hologram-mips"),
            size: wgpu::Extent3d {
                width: w0,
                height: h0,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MIP_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let level = |k: u32| {
            out.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: k,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut enc = ctx.encoder("fx-hologram-mips");
        for k in 0..levels {
            let input = if k == 0 {
                tex.create_view(&Default::default())
            } else {
                level(k - 1)
            };
            let output = level(k);
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-hologram-mip"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&output),
                    },
                ],
            });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-hologram-mip"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            let (lw, lh) = ((w0 >> k).max(1), (h0 >> k).max(1));
            pass.dispatch_workgroups(lw.div_ceil(8), lh.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}

/// Built on first use: most projects never draw a hologram.
pub(super) type LazyHologram = OnceLock<Arc<HologramFx>>;

/// Builds the other kernel (`first` was drawn) on a thread of its own. The
/// thread holds the kernels weakly, so it does nothing once the engine has
/// let them go.
#[cfg(not(target_arch = "wasm32"))]
fn build_the_other(fx: &Arc<HologramFx>, first: usize) {
    let fx: Weak<HologramFx> = Arc::downgrade(fx);
    // Without the thread the other kernel is still built when first drawn.
    let _ = std::thread::Builder::new()
        .name("hologram pipelines".into())
        .spawn(move || {
            if let Some(fx) = fx.upgrade() {
                fx.pipeline(first ^ 1);
            }
        });
}

#[cfg(target_arch = "wasm32")]
fn build_the_other(_fx: &Arc<HologramFx>, _first: usize) {}

impl FxEngine {
    /// Hologram over a linear working texture (premultiplied), returning a
    /// new one of the same size.
    pub fn hologram(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &HologramOp<'_>,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let mut first = false;
        let fx = self.hologram.get_or_init(|| {
            first = true;
            Arc::new(HologramFx::new(ctx))
        });
        // The bits Holographic foil's kernel reads, then the depth map (64)
        // and the backdrop (128).
        let flags = u32::from(op.matte.is_some())
            | u32::from(op.height_map.is_some()) << 1
            | u32::from(op.environment.is_some()) << 5
            | u32::from(op.depth_map.is_some()) << 6
            | u32::from(op.backdrop.is_some()) << 7;
        // The full kernel for wear or a map only it reads.
        let full = usize::from(flags & FULL_PICTURES != 0 || op.rows[ROW_WEAR][1] > 0.0);
        let pipeline = fx.pipeline(full);
        if first {
            build_the_other(fx, full);
        }
        let subject_mips = fx.halver.mips(ctx, src, w, h, false);
        let backdrop_mips = op
            .backdrop
            .map(|t| fx.halver.mips(ctx, t, t.width(), t.height(), false));
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-hologram-params"),
                contents: bytemuck::bytes_of(&Params {
                    flags: [flags, 0, 0, 0],
                    rows: op.rows,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let out = work_texture(ctx, w, h, "fx-hologram-out");
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        // An unset picture is bound to the input, and its flag stays clear.
        let views = [
            (0, view(src)),
            (2, view(&out)),
            (3, view(op.matte.unwrap_or(src))),
            (4, view(op.height_map.unwrap_or(src))),
            (8, view(op.environment.unwrap_or(src))),
            (9, view(op.depth_map.unwrap_or(src))),
            (10, view(op.backdrop.unwrap_or(src))),
            (11, view(&subject_mips)),
            (12, view(backdrop_mips.as_ref().unwrap_or(&subject_mips))),
        ];
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 1,
            resource: ubuf.as_entire_binding(),
        }];
        entries.extend(views.iter().map(|(binding, v)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: wgpu::BindingResource::TextureView(v),
        }));
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-hologram"),
            layout: &fx.layout,
            entries: &entries,
        });
        let mut enc = ctx.encoder("fx-hologram");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-hologram"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
