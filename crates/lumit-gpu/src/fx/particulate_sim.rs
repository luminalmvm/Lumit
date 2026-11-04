//! Particulate worked out step by step on the card (`fx_particulate_sim.wgsl`).
//!
//! The particles live in one buffer between steps, in px@comp, whatever raster
//! they are drawn at. A step moves them, counts who lives and who is born, and
//! places them by a prefix sum, so the result never depends on which thread got
//! there first. Drawing reads them through the closed forms' stream, so every
//! draw mode and the 3D scene take them as they come.

use wgpu::util::DeviceExt;

use super::particulate::{ParticulateOp, EVAL_WORKGROUP, SCAN_BLOCK};
use super::particulate_params::ParticulateParams;
use super::FxEngine;
use crate::GpuContext;

/// The bytes before the first particle.
const HEAD_BYTES: u64 = 16;
/// The bytes a particle takes between steps.
pub const PART_BYTES: u64 = 48;

/// The bytes one state of `cap` particles takes.
#[must_use]
pub fn state_bytes(cap: u32) -> u64 {
    HEAD_BYTES + u64::from(cap) * PART_BYTES
}

/// What a kind of collider is, as the kernel reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ColliderKind {
    /// Across and down, along `edges`, through every depth.
    Walls = 0,
    /// A rectangular card.
    Rect = 1,
    /// A card cut to the shapes `edges` hold.
    Shapes = 2,
    /// A floor with no end.
    Floor = 3,
    /// A box of distances in the buffer of them.
    Model = 4,
}

/// One thing the particles hit, flattened for the kernel.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SimCollider {
    /// This layer's pixels to the collider's own, by rows. For a model, to the
    /// cells of its box of distances.
    pub from: [[f32; 4]; 3],
    /// And back.
    pub to: [[f32; 4]; 3],
    /// Its box, in the pixels its edges are in.
    pub lo: [f32; 2],
    pub hi: [f32; 2],
    pub kind: u32,
    pub first: u32,
    pub n: u32,
    pub _pad: u32,
    /// A model's distances: where they start, and its cells each way.
    pub grid: [u32; 4],
}

/// One frame's unchanging part: the rows at its time, in px@comp, and what the
/// particles hit.
pub struct SimFrame<'a> {
    pub op: &'a ParticulateOp<'a>,
    /// The layer's picture size, px@comp, that the Edge row acts at.
    pub frame: (u32, u32),
    pub colliders: &'a [SimCollider],
    /// Every collider's edges, `[x0, y0, x1, y1]`.
    pub edges: &'a [[f32; 4]],
    /// The distances the Model colliders read, kept between frames.
    pub distances: Option<&'a wgpu::Buffer>,
}

/// One step's own numbers.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimStep {
    /// How long it is, seconds, and the walk's clock at its end.
    pub h: f32,
    pub clock: f32,
    /// The steady births it makes: index of the first, how many, when in the
    /// frame the first happens and how far apart, and when in the frame it ends.
    pub steady_first: u64,
    pub steady_n: u32,
    pub off0: f32,
    pub off_step: f32,
    pub end: f32,
    /// The burst it opens with: index of the first, how many.
    pub burst_first: u64,
    pub burst_n: u32,
    pub contact: u32,
    pub keep: f32,
    pub slip: f32,
    pub sparks: f32,
    pub spark_speed: f32,
    pub spark_life: f32,
    pub sparks_from: u32,
    pub axis: u32,
    pub attractor: f32,
    pub vortex: f32,
    /// The field's middle, and how far it reaches.
    pub field: [f32; 4],
    pub sphere: f32,
    /// How hard the flow pushes, px@comp per second².
    pub curl: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StepUniform {
    h: f32,
    clock: f32,
    cap: u32,
    blocks: u32,
    steady_first: [u32; 2],
    steady_n: u32,
    off0: f32,
    off_step: f32,
    end: f32,
    burst_n: u32,
    colliders: u32,
    burst_first: [u32; 2],
    contact: u32,
    keep: f32,
    slip: f32,
    sparks: f32,
    spark_speed: f32,
    spark_life: f32,
    sparks_from: u32,
    axis: u32,
    attract: f32,
    vortex: f32,
    field: [f32; 4],
    sphere: f32,
    curl: f32,
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OutUniform {
    px: f32,
    spark_size: f32,
    _pad: [f32; 2],
}

/// The particles on the card, ready for a [`ParticulateOp`] to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimView<'a> {
    pub state: &'a wgpu::Buffer,
    /// Raster pixels per px@comp.
    pub px: f32,
    /// A spark's size, as a share of its parent's.
    pub spark_size: f32,
}

/// The buffers one walk steps through.
pub struct SimWork {
    /// The state, between steps and after them.
    pub state: wgpu::Buffer,
    /// What each particle moved to, within a step.
    moved: wgpu::Buffer,
    counts: wgpu::Buffer,
    pub cap: u32,
}

impl SimWork {
    /// The bytes all three take.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.state.size() + self.moved.size() + self.counts.size()
    }
}

/// The buffer the Model colliders read their distances from.
#[must_use]
pub fn distances_buffer(ctx: &GpuContext, values: &[f32]) -> wgpu::Buffer {
    ctx.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fx-particulate-sim-distances"),
            contents: bytemuck::cast_slice(values),
            usage: wgpu::BufferUsages::STORAGE,
        })
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// The sim's layouts and pipelines, built the first time a sim runs.
pub(super) struct SimPipelines {
    main: wgpu::BindGroupLayout,
    step: wgpu::BindGroupLayout,
    out_layout: wgpu::BindGroupLayout,
    mover: wgpu::ComputePipeline,
    scan: wgpu::ComputePipeline,
    sums: wgpu::ComputePipeline,
    place: wgpu::ComputePipeline,
    birth: wgpu::ComputePipeline,
    out: wgpu::ComputePipeline,
    /// The picture births would follow, which nothing reads here.
    no_picture: wgpu::TextureView,
    /// A storage binding cannot be empty.
    nothing: wgpu::Buffer,
}

impl SimPipelines {
    fn new(fx: &FxEngine, ctx: &GpuContext) -> Self {
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fx-particulate-sim"),
                source: wgpu::ShaderSource::Wgsl(
                    [
                        include_str!("../fx_noise_core.wgsl"),
                        include_str!("../fx_blend_core.wgsl"),
                        include_str!("../fx_particulate.wgsl"),
                        include_str!("../fx_particulate_sim.wgsl"),
                    ]
                    .concat()
                    .into(),
                ),
            });
        let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        use wgpu::BufferBindingType as B;
        let storage = |read_only| B::Storage { read_only };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                })
        };
        let main = layout(
            "fx-particulate-sim-main",
            &[
                buffer(0, B::Uniform),
                buffer(3, storage(true)),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        );
        let step = layout(
            "fx-particulate-sim-step",
            &[
                buffer(0, B::Uniform),
                buffer(1, storage(false)),
                buffer(2, storage(false)),
                buffer(3, storage(false)),
                buffer(4, storage(true)),
                buffer(5, storage(true)),
                buffer(6, storage(true)),
            ],
        );
        let out_layout = layout(
            "fx-particulate-sim-out",
            &[buffer(10, storage(true)), buffer(11, B::Uniform)],
        );
        let empty = &fx.particulate_empty_layout;
        let step_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-sim"),
                bind_group_layouts: &[&main, empty, empty, &step],
                push_constant_ranges: &[],
            });
        let out_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-sim-out"),
                bind_group_layouts: &[&fx.particulate_layout, empty, empty, &out_layout],
                push_constant_ranges: &[],
            });
        let compute = |entry: &'static str, layout: &wgpu::PipelineLayout| {
            let (device, layout, module) = (ctx.device.clone(), layout.clone(), module.clone());
            crate::compiling::Compiling::start(move || {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
        };
        let started = ["sim_move", "sim_scan", "sim_sums", "sim_place", "sim_birth"]
            .map(|entry| compute(entry, &step_pl));
        let out = compute("sim_out", &out_pl);
        let no_picture = ctx
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("fx-particulate-sim-no-picture"),
                size: wgpu::Extent3d::default(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: ctx.working(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let nothing = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-sim-nothing"),
            size: 256,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let [mover, scan, sums, place, birth] = started.map(|c| c.wait());
        Self {
            main,
            step,
            out_layout,
            mover,
            scan,
            sums,
            place,
            birth,
            out: out.wait(),
            no_picture,
            nothing,
        }
    }
}

impl FxEngine {
    fn sim_pipelines(&self, ctx: &GpuContext) -> &SimPipelines {
        self.particulate_sim
            .get_or_init(|| SimPipelines::new(self, ctx))
    }

    /// The buffers a walk of `cap` particles steps through, with none born.
    pub fn sim_work(&self, ctx: &GpuContext, cap: u32) -> SimWork {
        use wgpu::BufferUsages as U;
        let buffer = |label, size, usage| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let blocks = u64::from(cap.div_ceil(SCAN_BLOCK));
        let state = buffer(
            "fx-particulate-sim-state",
            state_bytes(cap),
            U::STORAGE | U::COPY_DST | U::COPY_SRC,
        );
        let moved = buffer("fx-particulate-sim-moved", state_bytes(cap), U::STORAGE);
        let counts = buffer(
            "fx-particulate-sim-counts",
            (2 * u64::from(cap) + 2 * blocks + 4) * 4,
            U::STORAGE,
        );
        SimWork {
            state,
            moved,
            counts,
            cap,
        }
    }

    /// Everything but the frame's steps: no particles.
    pub fn sim_clear(&self, ctx: &GpuContext, work: &SimWork) {
        ctx.encoder("fx-particulate-sim-clear")
            .clear_buffer(&work.state, 0, Some(HEAD_BYTES));
    }

    /// A copy of where `work` has got to, for later.
    pub fn sim_keep(&self, ctx: &GpuContext, work: &SimWork) -> wgpu::Buffer {
        use wgpu::BufferUsages as U;
        let kept = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-sim-kept"),
            size: work.state.size(),
            usage: U::COPY_DST | U::COPY_SRC,
            mapped_at_creation: false,
        });
        ctx.encoder("fx-particulate-sim-keep")
            .copy_buffer_to_buffer(&work.state, 0, &kept, 0, work.state.size());
        kept
    }

    /// `work` back to a state [`Self::sim_keep`] kept.
    pub fn sim_restore(&self, ctx: &GpuContext, work: &SimWork, kept: &wgpu::Buffer) {
        ctx.encoder("fx-particulate-sim-restore")
            .copy_buffer_to_buffer(kept, 0, &work.state, 0, work.state.size().min(kept.size()));
    }

    /// `work`'s state as bytes, the count first and only the live particles,
    /// for writing to disk. Waits for the card. `None` when it would not give
    /// them back.
    pub fn sim_read(&self, ctx: &GpuContext, work: &SimWork) -> Option<Vec<u8>> {
        use wgpu::BufferUsages as U;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-sim-read"),
            size: work.state.size(),
            usage: U::COPY_DST | U::MAP_READ,
            mapped_at_creation: false,
        });
        ctx.encoder("fx-particulate-sim-read")
            .copy_buffer_to_buffer(&work.state, 0, &staging, 0, work.state.size());
        ctx.flush();
        let mut bytes: Vec<u8> =
            crate::device::read_buffer(ctx, ("particulate-sim", work.cap), &staging)?;
        let count = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
        bytes.truncate(state_bytes(count.min(work.cap)) as usize);
        Some(bytes)
    }

    /// `work`'s state from bytes [`Self::sim_read`] gave. `false` when they do
    /// not fit.
    pub fn sim_write(&self, ctx: &GpuContext, work: &SimWork, bytes: &[u8]) -> bool {
        if (bytes.len() as u64) > work.state.size() || bytes.len() < HEAD_BYTES as usize {
            return false;
        }
        let from = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-sim-write"),
                contents: bytes,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        ctx.encoder("fx-particulate-sim-write")
            .copy_buffer_to_buffer(&from, 0, &work.state, 0, bytes.len() as u64);
        true
    }

    /// `steps`, in order, on `work`.
    pub fn sim_steps(
        &self,
        ctx: &GpuContext,
        work: &SimWork,
        frame: &SimFrame<'_>,
        steps: &[SimStep],
    ) {
        let set = self.sim_pipelines(ctx);
        let device = &ctx.device;
        let init = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let params = ParticulateParams::of(frame.op, frame.frame, (1.0, 1.0), 0);
        let params = init(
            "fx-particulate-sim-params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let storage = wgpu::BufferUsages::STORAGE;
        let path: &[[f32; 4]] = if frame.op.path.is_empty() {
            &[[0.0; 4]]
        } else {
            frame.op.path
        };
        let path = init(
            "fx-particulate-sim-path",
            bytemuck::cast_slice(path),
            storage,
        );
        let colliders = (!frame.colliders.is_empty()).then(|| {
            init(
                "fx-particulate-sim-colliders",
                bytemuck::cast_slice(frame.colliders),
                storage,
            )
        });
        let edges = (!frame.edges.is_empty()).then(|| {
            init(
                "fx-particulate-sim-edges",
                bytemuck::cast_slice(frame.edges),
                storage,
            )
        });
        let main = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-sim-main"),
            layout: &set.main,
            entries: &[
                entry(0, &params),
                entry(3, &path),
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&set.no_picture),
                },
            ],
        });
        let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-sim-empty"),
            layout: &self.particulate_empty_layout,
            entries: &[],
        });
        let per_particle = work.cap.div_ceil(EVAL_WORKGROUP);
        let blocks = work.cap.div_ceil(SCAN_BLOCK);
        let wide = |v: u64| [v as u32, (v >> 32) as u32];
        let mut enc = ctx.encoder("fx-particulate-sim");
        for s in steps {
            let uniform = StepUniform {
                h: s.h,
                clock: s.clock,
                cap: work.cap,
                blocks,
                steady_first: wide(s.steady_first),
                steady_n: s.steady_n,
                off0: s.off0,
                off_step: s.off_step,
                end: s.end,
                burst_n: s.burst_n,
                colliders: frame.colliders.len() as u32,
                burst_first: wide(s.burst_first),
                contact: s.contact,
                keep: s.keep,
                slip: s.slip,
                sparks: s.sparks,
                spark_speed: s.spark_speed,
                spark_life: s.spark_life,
                sparks_from: s.sparks_from,
                axis: s.axis,
                attract: s.attractor,
                vortex: s.vortex,
                field: s.field,
                sphere: s.sphere,
                curl: s.curl,
                _pad: [0.0; 2],
            };
            let uniform = init(
                "fx-particulate-sim-step",
                bytemuck::bytes_of(&uniform),
                wgpu::BufferUsages::UNIFORM,
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-particulate-sim-step"),
                layout: &set.step,
                entries: &[
                    entry(0, &uniform),
                    entry(1, &work.state),
                    entry(2, &work.moved),
                    entry(3, &work.counts),
                    entry(4, colliders.as_ref().unwrap_or(&set.nothing)),
                    entry(5, edges.as_ref().unwrap_or(&set.nothing)),
                    entry(6, frame.distances.unwrap_or(&set.nothing)),
                ],
            });
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-particulate-sim"),
                timestamp_writes: None,
            });
            cp.set_bind_group(0, &main, &[]);
            cp.set_bind_group(1, &empty, &[]);
            cp.set_bind_group(2, &empty, &[]);
            cp.set_bind_group(3, &group, &[]);
            cp.set_pipeline(&set.mover);
            cp.dispatch_workgroups(per_particle, 1, 1);
            cp.set_pipeline(&set.scan);
            cp.dispatch_workgroups(blocks, 1, 1);
            cp.set_pipeline(&set.sums);
            cp.dispatch_workgroups(1, 1, 1);
            cp.set_pipeline(&set.place);
            cp.dispatch_workgroups(per_particle, 1, 1);
            let born = (s.steady_n + s.burst_n).min(work.cap);
            if born > 0 {
                cp.set_pipeline(&set.birth);
                cp.dispatch_workgroups(born.div_ceil(EVAL_WORKGROUP), 1, 1);
            }
        }
    }

    /// The bind group that turns `view` into the closed forms' stream.
    pub(super) fn sim_out_bind(&self, ctx: &GpuContext, view: &SimView<'_>) -> wgpu::BindGroup {
        let set = self.sim_pipelines(ctx);
        let uniform = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-sim-out"),
                contents: bytemuck::bytes_of(&OutUniform {
                    px: view.px,
                    spark_size: view.spark_size,
                    _pad: [0.0; 2],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-sim-out"),
            layout: &set.out_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: view.state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }

    /// Runs the pass [`Self::sim_out_bind`] was made for, on `cp`, which already
    /// holds the closed forms' own bindings in its first group.
    pub(super) fn sim_out_pass(
        &self,
        cp: &mut wgpu::ComputePass<'_>,
        empty: &wgpu::BindGroup,
        out: &wgpu::BindGroup,
        cap: u32,
    ) {
        if let Some(set) = self.particulate_sim.get() {
            cp.set_bind_group(1, empty, &[]);
            cp.set_bind_group(2, empty, &[]);
            cp.set_bind_group(3, out, &[]);
            cp.set_pipeline(&set.out);
            cp.dispatch_workgroups(cap.div_ceil(EVAL_WORKGROUP), 1, 1);
        }
    }
}
