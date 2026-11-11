//! Denoise's kernels (`fx_denoise*.wgsl`), the GPU half of
//! `lumit_core::fx::denoise`.
//!
//! Kept apart from [`super::FxEngine`] with layouts of their own: a pass reads
//! up to five pictures at once, and the temporal sums write two. Built once a
//! device by whoever runs them. Runs on the GPU-submit thread.
//!
//! The working pictures are Y'CbCr with negative chroma, so they are kept in a
//! float format whatever the project's depth: the working one when it is float,
//! half float when it is eight bits. The temporal sums are 32-bit float, one
//! pair of them for any number of frames.
//!
//! They are Denoise's own and kept from one frame to the next, charged to the
//! governor while they are and let go when it is short of memory
//! ([`DenoiseKernels::let_go`]), rather than taken from the frame's pool and made
//! again every frame: the frame's pool lets its memory go when the frame
//! ends, and a frame-sized set made again from nothing costs the card a new
//! block of memory nearly every frame. None of them is ever handed to the
//! frame's pool, so no other pass can be drawing in one when the next run
//! takes it.

use crate::GpuContext;

use super::work_texture;

/// What the passes read: `lumit_core::fx::denoise::DenoiseParams`, number for
/// number (lumit-gpu cannot name lumit-core outside its tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DenoiseOp {
    pub view: u32,
    pub luma_sigma: f32,
    pub chroma_sigma: f32,
    pub tone: [f32; 3],
    pub luma_bands: [f32; 4],
    pub chroma_bands: [f32; 4],
    pub frames: u32,
    pub t0: f32,
    pub t1: f32,
    pub strength: f32,
    pub luma_amount: f32,
    pub chroma_amount: f32,
    pub band_amounts: [f32; 4],
    pub detail: f32,
    pub sharpen: f32,
    pub grain: f32,
    pub mix: f32,
}

/// One neighbour frame: its picture (premultiplied linear, the size of the
/// frame) and the motion from this frame to it at the size it was measured
/// at, `.xy` the vectors in its own pixels and `.z` the confidence. `None` reads
/// it in place at full confidence, which is Follow motion off.
pub struct Neighbour<'a> {
    pub picture: &'a wgpu::Texture,
    pub field: Option<&'a wgpu::Texture>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    dims: [u32; 4],
    pass: [u32; 4],
    sig: [f32; 4],
    tone: [f32; 4],
    lband: [f32; 4],
    cband: [f32; 4],
    bamt: [f32; 4],
    fin: [f32; 4],
    fin2: [f32; 4],
}

/// A field is bound to this pass.
const HAS_FIELD: u32 = 1;
/// The finest band: there is no sum before it.
const FIRST_BAND: u32 = 2;
/// A matte is bound to the last pass.
const HAS_MATTE: u32 = 4;

impl Params {
    fn of(op: &DenoiseOp, w: u32, h: u32) -> Self {
        Self {
            dims: [w, h, 0, 0],
            pass: [0, 0, op.view, 0],
            sig: [op.luma_sigma, op.chroma_sigma, op.t0, op.t1],
            tone: [op.tone[0], op.tone[1], op.tone[2], op.strength],
            lband: op.luma_bands,
            cband: op.chroma_bands,
            bamt: op.band_amounts,
            fin: [op.luma_amount, op.chroma_amount, op.detail, op.sharpen],
            fin2: [op.grain, op.mix, op.frames as f32, 0.0],
        }
    }
}

/// One module's layout and its entry points.
struct Module {
    layout: wgpu::BindGroupLayout,
    pipes: Vec<(&'static str, wgpu::ComputePipeline)>,
}

impl Module {
    fn new(
        ctx: &GpuContext,
        label: &str,
        text: &str,
        outs: &[wgpu::TextureFormat],
        entries: &[&'static str],
    ) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}{text}", include_str!("../fx_denoise_common.wgsl")).into(),
            ),
        });
        let mut layout_entries: Vec<wgpu::BindGroupLayoutEntry> = (0..6)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            })
            .collect();
        for (i, format) in outs.iter().enumerate() {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 6 + i as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: *format,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            });
        }
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 8,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &layout_entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipes = entries
            .iter()
            .map(|entry| {
                (
                    *entry,
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&pipeline_layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                )
            })
            .collect();
        Self { layout, pipes }
    }

    /// Run `entry` over `w` by `h`, reading `inputs` (unused slots take any
    /// picture) and writing `outs`.
    fn run(
        &self,
        ctx: &GpuContext,
        entry: &str,
        inputs: [&wgpu::Texture; 6],
        outs: &[&wgpu::Texture],
        params: &Params,
    ) {
        use wgpu::util::DeviceExt;
        let Some((_, pipe)) = self.pipes.iter().find(|(e, _)| *e == entry) else {
            return;
        };
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-denoise-params"),
                contents: bytemuck::bytes_of(params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let views: Vec<wgpu::TextureView> = inputs
            .iter()
            .chain(outs.iter())
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let mut entries: Vec<wgpu::BindGroupEntry<'_>> = views
            .iter()
            .enumerate()
            .map(|(i, v)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(v),
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: 8,
            resource: ubuf.as_entire_binding(),
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-denoise-bind"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut enc = ctx.encoder("fx-denoise-enc");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(entry),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(params.dims[0].div_ceil(8), params.dims[1].div_ceil(8), 1);
        }
        drop(enc);
    }
}

/// Denoise's pipelines on one device.
pub struct DenoiseKernels {
    mid: Module,
    sum: Module,
    out: Module,
    /// The format the working pictures are kept in.
    mid_format: wgpu::TextureFormat,
    /// The working format the last pass writes, which they were built for.
    working: wgpu::TextureFormat,
    /// The working pictures not in use, and what the governor holds for them.
    spare: std::cell::RefCell<Spare>,
}

/// Pictures a run gave back, kept for the next.
#[derive(Default)]
struct Spare {
    free: Vec<wgpu::Texture>,
    held: Option<lumit_budget::Reservation>,
}

impl DenoiseKernels {
    pub fn new(ctx: &GpuContext) -> Self {
        let mid_format = match ctx.working() {
            f @ (wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float) => f,
            _ => wgpu::TextureFormat::Rgba16Float,
        };
        let mid_text = include_str!("../fx_denoise.wgsl");
        let mid_text = if mid_format == wgpu::TextureFormat::Rgba32Float {
            mid_text.replace(
                "var out_mid: texture_storage_2d<rgba16float,",
                "var out_mid: texture_storage_2d<rgba32float,",
            )
        } else {
            mid_text.to_owned()
        };
        Self {
            mid: Module::new(
                ctx,
                "fx-denoise",
                &mid_text,
                &[mid_format],
                &[
                    "prep", "warp", "resolve", "plain", "atrous_h", "atrous_v", "shrink",
                ],
            ),
            sum: Module::new(
                ctx,
                "fx-denoise-sum",
                include_str!("../fx_denoise_sum.wgsl"),
                &[
                    wgpu::TextureFormat::Rgba32Float,
                    wgpu::TextureFormat::R32Float,
                ],
                &["init", "accumulate"],
            ),
            out: Module::new(
                ctx,
                "fx-denoise-finish",
                &super::at_depth(ctx.working(), include_str!("../fx_denoise_finish.wgsl")),
                &[ctx.working()],
                &["finish"],
            ),
            mid_format,
            working: ctx.working(),
            spare: std::cell::RefCell::default(),
        }
    }

    /// The working format these were built for: a context working in
    /// another needs kernels of its own.
    #[must_use]
    pub fn working(&self) -> wgpu::TextureFormat {
        self.working
    }

    /// A working picture, a spare one when one matches.
    fn mid(&self, ctx: &GpuContext, w: u32, h: u32) -> wgpu::Texture {
        self.picture(ctx, w, h, self.mid_format, "fx-denoise-mid")
    }

    /// A picture `format` and `w` by `h`: a spare one, or a new one charged to
    /// the frame like any work texture.
    fn picture(
        &self,
        ctx: &GpuContext,
        w: u32,
        h: u32,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> wgpu::Texture {
        {
            let mut spare = self.spare.borrow_mut();
            if let Some(i) = spare
                .free
                .iter()
                .position(|t| t.width() == w && t.height() == h && t.format() == format)
            {
                return spare.free.swap_remove(i);
            }
        }
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
            usage: crate::WORK_USAGE,
            view_formats: &[],
        })
    }

    /// Give a picture back for a later pass, or the next run, to use.
    fn done(&self, t: wgpu::Texture) {
        self.spare.borrow_mut().free.push(t);
    }

    /// After a run: keep what it gave back if the governor has room, holding
    /// that much for as long as it is kept, and let it all go if not. A
    /// picture left over from a run at another size goes too.
    fn settle(&self, ctx: &GpuContext, w: u32, h: u32) {
        let mut spare = self.spare.borrow_mut();
        spare.free.retain(|t| t.width() == w && t.height() == h);
        let bytes: u64 = spare
            .free
            .iter()
            .map(|t| crate::texture_bytes(t.format(), t.width(), t.height()))
            .sum();
        spare.held = None;
        let pressed = ctx
            .ledger()
            .pressure(lumit_budget::Tier::Vram)
            .should_trim();
        spare.held = if pressed {
            None
        } else {
            ctx.ledger().try_reserve(lumit_budget::Tier::Vram, bytes)
        };
        if spare.held.is_none() {
            spare.free.clear();
        }
    }

    /// Under memory pressure: the spare pictures the kept kernels hold go
    /// back to the card, and the governor's hold on them with them. The
    /// kernels themselves stay.
    pub fn let_go(fx: &super::FxEngine) {
        if let Some(kernels) = fx.take_kept::<Self>() {
            {
                let mut spare = kernels.spare.borrow_mut();
                spare.free.clear();
                spare.held = None;
            }
            fx.keep(kernels);
        }
    }

    /// `input` (premultiplied linear, `w` by `h`) denoised, with `neighbours`
    /// averaged in, in the order given. `matte`, the picture's size, scales
    /// the reduction per pixel by its luma.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        ctx: &GpuContext,
        input: &wgpu::Texture,
        w: u32,
        h: u32,
        neighbours: &[Neighbour<'_>],
        op: &DenoiseOp,
        matte: Option<&wgpu::Texture>,
    ) -> wgpu::Texture {
        let base = Params::of(op, w, h);
        let current = self.mid(ctx, w, h);
        self.mid.run(ctx, "prep", [input; 6], &[&current], &base);
        let averaged = self.average(ctx, &current, w, h, neighbours, op, &base);
        // The bands, finest first, each shrunk into the running sum.
        let mut sum: Option<wgpu::Texture> = None;
        let mut level: Option<wgpu::Texture> = None;
        for band in 0..4u32 {
            let above = level.as_ref().unwrap_or(&averaged);
            let mut p = base;
            p.pass[0] = 1 << band;
            let across = self.mid(ctx, w, h);
            self.mid.run(ctx, "atrous_h", [above; 6], &[&across], &p);
            let below = self.mid(ctx, w, h);
            self.mid.run(ctx, "atrous_v", [&across; 6], &[&below], &p);
            self.done(across);
            p.pass[1] = band;
            p.pass[3] = if sum.is_none() { FIRST_BAND } else { 0 };
            let next_sum = self.mid(ctx, w, h);
            {
                let before = sum.as_ref().unwrap_or(&current);
                self.mid.run(
                    ctx,
                    "shrink",
                    [above, &below, &averaged, &current, before, &current],
                    &[&next_sum],
                    &p,
                );
            }
            if let Some(old) = sum.replace(next_sum) {
                self.done(old);
            }
            if let Some(old) = level.replace(below) {
                self.done(old);
            }
        }
        let out = work_texture(ctx, w, h, "fx-denoise-out");
        if let (Some(sum), Some(coarse)) = (sum, level) {
            let mut p = base;
            let matte = matte.filter(|m| m.width() == w && m.height() == h);
            if matte.is_some() {
                p.pass[3] = HAS_MATTE;
            }
            self.out.run(
                ctx,
                "finish",
                [
                    input,
                    &current,
                    &sum,
                    &coarse,
                    &averaged,
                    matte.unwrap_or(input),
                ],
                &[&out],
                &p,
            );
            self.done(sum);
            self.done(coarse);
        }
        self.done(current);
        self.done(averaged);
        self.settle(ctx, w, h);
        out
    }

    /// The current frame with every neighbour lined up and averaged in, the
    /// frames blended in `.w`; the current frame alone with none.
    #[allow(clippy::too_many_arguments)]
    fn average(
        &self,
        ctx: &GpuContext,
        current: &wgpu::Texture,
        w: u32,
        h: u32,
        neighbours: &[Neighbour<'_>],
        op: &DenoiseOp,
        base: &Params,
    ) -> wgpu::Texture {
        let averaged = self.mid(ctx, w, h);
        let usable: Vec<&Neighbour<'_>> = neighbours
            .iter()
            .filter(|n| n.picture.width() == w && n.picture.height() == h)
            .collect();
        if op.frames == 0 || usable.is_empty() {
            self.mid.run(ctx, "plain", [current; 6], &[&averaged], base);
            return averaged;
        }
        let sums = |label| self.picture(ctx, w, h, wgpu::TextureFormat::Rgba32Float, label);
        let squares = |label| self.picture(ctx, w, h, wgpu::TextureFormat::R32Float, label);
        let mut s = (sums("fx-denoise-sum-a"), squares("fx-denoise-sq-a"));
        let mut t = (sums("fx-denoise-sum-b"), squares("fx-denoise-sq-b"));
        self.sum.run(ctx, "init", [current; 6], &[&s.0, &s.1], base);
        let aligned = self.mid(ctx, w, h);
        for n in usable {
            let mut p = *base;
            if let Some(f) = n.field {
                p.dims[2] = f.width();
                p.dims[3] = f.height();
                p.pass[3] = HAS_FIELD;
            }
            let field = n.field.unwrap_or(n.picture);
            self.mid.run(
                ctx,
                "warp",
                [n.picture, field, n.picture, n.picture, n.picture, n.picture],
                &[&aligned],
                &p,
            );
            self.sum.run(
                ctx,
                "accumulate",
                [current, &aligned, &s.0, &s.1, current, current],
                &[&t.0, &t.1],
                base,
            );
            std::mem::swap(&mut s, &mut t);
        }
        self.done(aligned);
        self.mid.run(
            ctx,
            "resolve",
            [&s.0, &s.1, &s.0, &s.0, &s.0, &s.0],
            &[&averaged],
            base,
        );
        for t in [s.0, s.1, t.0, t.1] {
            self.done(t);
        }
        averaged
    }
}
