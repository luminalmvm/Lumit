//! Timewarp's kernels: the crossfade of two moments and the frame made from
//! the motion between them (`fx_timewarp.wgsl`).
//!
//! Kept apart from [`super::FxEngine`] with their own bind layout: the
//! synthesis reads two pictures, two fields and a mask at once, a shape no
//! other kernel shares. Built once a device by whoever runs them.

use crate::GpuContext;

use super::work_texture;

/// Timewarp's pipelines on one device.
pub struct TimewarpKernels {
    layout: wgpu::BindGroupLayout,
    mix: wgpu::ComputePipeline,
    occ: wgpu::ComputePipeline,
    grow: wgpu::ComputePipeline,
    synth: wgpu::ComputePipeline,
    field: wgpu::ComputePipeline,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    phase: f32,
    mix_amt: f32,
    w: u32,
    h: u32,
    fw: u32,
    fh: u32,
    _pad: [u32; 2],
}

/// One pass: the picture's size, the motion's, and the size it runs over.
struct Sizes {
    picture: (u32, u32),
    motion: (u32, u32),
    over: (u32, u32),
}

impl TimewarpKernels {
    pub fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-timewarp"),
            source: wgpu::ShaderSource::Wgsl(
                super::at_depth(ctx.working(), include_str!("../fx_timewarp.wgsl")).into(),
            ),
        });
        let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-timewarp-layout"),
            entries: &[
                texture(0),
                texture(1),
                texture(2),
                texture(3),
                texture(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: ctx.working(),
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-timewarp-pipeline-layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            mix: pipeline("tw_mix"),
            occ: pipeline("tw_occ"),
            grow: pipeline("tw_grow"),
            synth: pipeline("tw_synth"),
            field: pipeline("tw_field"),
            layout,
        }
    }

    /// `a` towards `b` by `amount`, both `w` by `h`.
    pub fn mix(
        &self,
        ctx: &GpuContext,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        amount: f32,
    ) -> wgpu::Texture {
        let (w, h) = (a.width(), a.height());
        let out = work_texture(ctx, w, h, "fx-timewarp-mix");
        let sizes = Sizes {
            picture: (w, h),
            motion: (w, h),
            over: (w, h),
        };
        self.run(ctx, &self.mix, [a, b, a, a, a], &out, &sizes, 0.0, amount);
        out
    }

    /// The frame at `phase` between `a` and `b`, made from the motion `fwd`
    /// (a to b) and `bwd` (b to a), each at the size it was measured at, its
    /// vectors in measured pixels.
    pub fn synth(
        &self,
        ctx: &GpuContext,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        fwd: &wgpu::Texture,
        bwd: &wgpu::Texture,
        phase: f32,
    ) -> wgpu::Texture {
        let (w, h) = (a.width(), a.height());
        let (fw, fh) = (fwd.width(), fwd.height());
        let at_motion = Sizes {
            picture: (w, h),
            motion: (fw, fh),
            over: (fw, fh),
        };
        let raw = work_texture(ctx, fw, fh, "fx-timewarp-occ");
        self.run(
            ctx,
            &self.occ,
            [a, b, fwd, bwd, a],
            &raw,
            &at_motion,
            phase,
            0.0,
        );
        let grown = work_texture(ctx, fw, fh, "fx-timewarp-occ-grown");
        self.run(
            ctx,
            &self.grow,
            [a, b, fwd, bwd, &raw],
            &grown,
            &at_motion,
            phase,
            0.0,
        );
        let out = work_texture(ctx, w, h, "fx-timewarp-synth");
        let at_picture = Sizes {
            over: (w, h),
            ..at_motion
        };
        self.run(
            ctx,
            &self.synth,
            [a, b, fwd, bwd, &grown],
            &out,
            &at_picture,
            phase,
            0.0,
        );
        out
    }

    /// The motion `fwd` brought up to `w` by `h`, in picture pixels, for the
    /// motion blur.
    pub fn field(&self, ctx: &GpuContext, fwd: &wgpu::Texture, w: u32, h: u32) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-timewarp-field");
        let sizes = Sizes {
            picture: (w, h),
            motion: (fwd.width(), fwd.height()),
            over: (w, h),
        };
        self.run(
            ctx,
            &self.field,
            [fwd, fwd, fwd, fwd, fwd],
            &out,
            &sizes,
            0.0,
            0.0,
        );
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        inputs: [&wgpu::Texture; 5],
        out: &wgpu::Texture,
        sizes: &Sizes,
        phase: f32,
        mix_amt: f32,
    ) {
        use wgpu::util::DeviceExt;
        let ((w, h), (fw, fh)) = (sizes.picture, sizes.motion);
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-timewarp-params"),
                contents: bytemuck::bytes_of(&Params {
                    phase,
                    mix_amt,
                    w,
                    h,
                    fw,
                    fh,
                    _pad: [0; 2],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let views: Vec<wgpu::TextureView> = inputs
            .iter()
            .chain(std::iter::once(&out))
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
            binding: 6,
            resource: ubuf.as_entire_binding(),
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-timewarp-bind"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut enc = ctx.encoder("fx-timewarp-enc");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-timewarp-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(sizes.over.0.div_ceil(8), sizes.over.1.div_ceil(8), 1);
        }
        drop(enc);
    }
}
