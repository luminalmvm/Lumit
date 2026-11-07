//! Grid smoke and fire worked out on the card (`fx_smoke.wgsl`).
//!
//! The state is one buffer of cells. A step runs a fixed list of passes over the
//! whole grid, so it takes the same work and comes out the same whatever is in
//! it. For drawing, density and heat are copied into a 3D texture the marcher
//! reads.

use wgpu::util::DeviceExt;

use super::FxEngine;
use crate::GpuContext;

/// The bytes a cell of state takes, speed and fields.
const CELL_BYTES: u64 = 16;

/// The bytes a cell of a walk's working buffers takes: three of state, two of
/// pressure, one of spread, and density and heat twice over for the marcher.
const WORK_CELL_BYTES: u64 = 3 * CELL_BYTES + 3 * 4 + 2 * 4;

/// How far a uniform is from the next.
const UNIFORM_STEP: u64 = 256;

/// How many cells a grid of `cells` holds.
#[must_use]
pub fn cell_count(cells: [u32; 3]) -> u64 {
    cells.iter().map(|c| u64::from(*c)).product()
}

/// The bytes one state of `cells` takes.
#[must_use]
pub fn state_bytes(cells: [u32; 3]) -> u64 {
    cell_count(cells) * CELL_BYTES
}

/// The bytes a walk of `cells` holds to step through.
#[must_use]
pub fn work_bytes(cells: [u32; 3]) -> u64 {
    cell_count(cells) * WORK_CELL_BYTES
}

/// What a walk emits from, as the kernel reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EmitKind {
    Box = 0,
    Particles = 1,
    Model = 2,
}

/// A step's numbers, each already in cells and seconds.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SmokeStep {
    pub cells: [f32; 4],
    pub wind: [f32; 4],
    pub emit: [f32; 4],
    pub emit_middle: [f32; 4],
    pub emit_reach: [f32; 4],
    pub motion: [f32; 4],
    pub burn: [f32; 4],
    pub fire: [f32; 4],
    pub to_model: [[f32; 4]; 3],
    pub model_cells: [f32; 4],
    pub extra: [f32; 4],
    pub more: [f32; 4],
    pub drift: [f32; 4],
}

/// A layer or a model the gas goes round, as the kernel reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SmokeSolid {
    /// Cells of the grid to the solid's own space, by rows.
    pub from_cell: [[f32; 4]; 3],
    /// Cells of the grid to where that part of it was at the frame's start.
    pub was: [[f32; 4]; 3],
    /// A card's box in its own space.
    pub lo: [f32; 4],
    pub hi: [f32; 4],
    /// A model's distances: where they start, then its cells each way.
    pub grid: [u32; 4],
    /// 0 a card, 1 a model.
    pub kind: [u32; 4],
}

/// What a walk emits from beyond its numbers.
pub struct SmokeEmit<'a> {
    /// Every model's distances the walk reads, one after another, negative
    /// inside.
    pub distances: Option<&'a [f32]>,
    /// Whether particles are laid into the grid each frame.
    pub particles: bool,
    /// Whether gas goes round layers or models.
    pub solids: bool,
}

/// A walk's buffers, which only ever change by a step or a state put back.
pub struct SmokeWork {
    pub cells: [u32; 3],
    /// The state between steps is in the first.
    state: [wgpu::Buffer; 3],
    emitted: wgpu::Buffer,
    /// Density and heat for the marcher, the buffer they are laid out in, and
    /// how many frames ago they were last brought up to date.
    drawn: wgpu::Texture,
    drawn_rows: wgpu::Buffer,
    /// What models' distances the walk reads, and each cell's solid this frame.
    distances: wgpu::Buffer,
    solid: wgpu::Buffer,
    passes: Passes,
}

/// A step's passes, bound once.
struct Passes {
    curl: wgpu::BindGroup,
    force: wgpu::BindGroup,
    spread: wgpu::BindGroup,
    solve: [wgpu::BindGroup; 2],
    settle: wgpu::BindGroup,
    carry: wgpu::BindGroup,
    sharpen: wgpu::BindGroup,
    drawn: wgpu::BindGroup,
}

impl SmokeWork {
    /// The bytes it holds.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        work_bytes(self.cells) + self.emitted.size()
    }

    /// Density in red and heat in green, for the marcher, as of the last
    /// [`FxEngine::smoke_draw`].
    #[must_use]
    pub fn fields(&self) -> &wgpu::Texture {
        &self.drawn
    }
}

/// The kernels' layouts and pipelines, built the first time smoke runs.
pub(super) struct SmokePipelines {
    step_layout: wgpu::BindGroupLayout,
    curl: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    force: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    spread: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    solve: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    settle: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    carry: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    sharpen: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    drawn: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    solids: (wgpu::BindGroupLayout, wgpu::ComputePipeline),
    splat_layout: wgpu::BindGroupLayout,
    splat: wgpu::ComputePipeline,
}

fn buffer_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl SmokePipelines {
    fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-smoke"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("../fx_noise_core.wgsl"),
                    include_str!("../fx_smoke.wgsl")
                )
                .into(),
            ),
        });
        let step_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-smoke-step"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pass = |entry: &str, read: &[u32], written: &[u32]| {
            let entries: Vec<_> = read
                .iter()
                .map(|b| buffer_entry(*b, true))
                .chain(written.iter().map(|b| buffer_entry(*b, false)))
                .collect();
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(entry),
                entries: &entries,
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[&layout, &step_layout],
                push_constant_ranges: &[],
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
            (layout, pipeline)
        };
        let splat_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-smoke-splat"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../fx_smoke_splat.wgsl").into()),
        });
        let splat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-smoke-splat"),
            entries: &[
                buffer_entry(0, true),
                buffer_entry(1, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
        let splat_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-smoke-splat"),
                bind_group_layouts: &[&splat_layout],
                push_constant_ranges: &[],
            });
        Self {
            curl: pass("smoke_curl", &[0], &[2]),
            force: pass("smoke_force", &[0, 1, 7, 8, 10], &[2]),
            spread: pass("smoke_spread", &[0], &[5, 6]),
            solve: pass("smoke_solve", &[3, 4, 10], &[5]),
            settle: pass("smoke_settle", &[0, 3, 10], &[2]),
            carry: pass("smoke_carry", &[0], &[2]),
            sharpen: pass("smoke_sharpen", &[0, 1, 10], &[2]),
            drawn: pass("smoke_drawn", &[0], &[9]),
            solids: pass("smoke_solids", &[7, 12], &[11]),
            splat: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("smoke_splat"),
                layout: Some(&splat_pipeline_layout),
                module: &splat_module,
                entry_point: Some("smoke_splat"),
                compilation_options: Default::default(),
                cache: None,
            }),
            splat_layout,
            step_layout,
        }
    }
}

/// The cells of a row of [`SmokeWork::fields`]' buffer, which a copy wants a
/// round number of bytes.
fn drawn_pitch(width: u32) -> u32 {
    (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) / 4
}

/// A 3D field of density and heat `cells` big, that a buffer is copied into.
#[must_use]
pub fn field_texture(ctx: &GpuContext, cells: [u32; 3]) -> wgpu::Texture {
    use wgpu::TextureUsages as U;
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fx-smoke-drawn"),
        size: wgpu::Extent3d {
            width: cells[0],
            height: cells[1],
            depth_or_array_layers: cells[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rg16Float,
        usage: U::TEXTURE_BINDING | U::COPY_DST,
        view_formats: &[],
    })
}

/// A 3D field `cells` big from half floats, density then heat, a row of `pitch`
/// cells at a time.
#[must_use]
pub fn field_from(ctx: &GpuContext, cells: [u32; 3], pitch: u32, values: &[u16]) -> wgpu::Texture {
    let texture = field_texture(ctx, cells);
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pitch * 4),
            rows_per_image: Some(cells[1]),
        },
        texture.size(),
    );
    texture
}

/// How many times a step's solve goes round for a grid `longest` cells long:
/// fixed, and never stopping early, so a step always takes the same work. Enough
/// to carry a push about a quarter of the way across.
#[must_use]
pub fn solve_rounds(longest: u32) -> u32 {
    (longest / 4).clamp(16, 64) & !1
}

impl FxEngine {
    fn smoke_pipes(&self, ctx: &GpuContext) -> &SmokePipelines {
        self.smoke.get_or_init(|| SmokePipelines::new(ctx))
    }

    /// The buffers a walk of `cells` steps through, with nothing in them.
    pub fn smoke_work(&self, ctx: &GpuContext, cells: [u32; 3], emit: &SmokeEmit<'_>) -> SmokeWork {
        use wgpu::BufferUsages as U;
        let fx = self.smoke_pipes(ctx);
        let buffer = |label, size, usage| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let n = cell_count(cells);
        let state = ["fx-smoke-state", "fx-smoke-on", "fx-smoke-out"].map(|label| {
            buffer(
                label,
                n * CELL_BYTES,
                U::STORAGE | U::COPY_DST | U::COPY_SRC,
            )
        });
        let pressure = ["fx-smoke-pressure", "fx-smoke-pressure-on"]
            .map(|label| buffer(label, n * 4, U::STORAGE));
        let spread = buffer("fx-smoke-spread", n * 4, U::STORAGE);
        let emitted = buffer(
            "fx-smoke-emitted",
            if emit.particles { n * 4 } else { 4 },
            U::STORAGE | U::COPY_DST,
        );
        let pitch = u64::from(drawn_pitch(cells[0]));
        let drawn_rows = buffer(
            "fx-smoke-drawn-rows",
            pitch * u64::from(cells[1]) * u64::from(cells[2]) * 4,
            U::STORAGE | U::COPY_SRC,
        );
        let distances = match emit.distances {
            Some(values) if !values.is_empty() => {
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("fx-smoke-distances"),
                        contents: bytemuck::cast_slice(values),
                        usage: U::STORAGE,
                    })
            }
            _ => buffer("fx-smoke-no-distances", 4, U::STORAGE),
        };
        let solid = buffer(
            "fx-smoke-solid",
            if emit.solids { n * 8 } else { 8 },
            U::STORAGE,
        );
        let group = |layout: &wgpu::BindGroupLayout, buffers: &[(u32, &wgpu::Buffer)]| {
            let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
                .iter()
                .map(|(binding, b)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: b.as_entire_binding(),
                })
                .collect();
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx-smoke-pass"),
                layout,
                entries: &entries,
            })
        };
        let [a, b, c] = &state;
        // Which buffer is which at each pass: see `smoke_steps`.
        let passes = Passes {
            curl: group(&fx.curl.0, &[(0, a), (2, c)]),
            force: group(
                &fx.force.0,
                &[
                    (0, a),
                    (1, c),
                    (7, &distances),
                    (8, &emitted),
                    (10, &solid),
                    (2, b),
                ],
            ),
            spread: group(&fx.spread.0, &[(0, b), (5, &spread), (6, &pressure[0])]),
            solve: [
                group(
                    &fx.solve.0,
                    &[
                        (3, &pressure[0]),
                        (4, &spread),
                        (10, &solid),
                        (5, &pressure[1]),
                    ],
                ),
                group(
                    &fx.solve.0,
                    &[
                        (3, &pressure[1]),
                        (4, &spread),
                        (10, &solid),
                        (5, &pressure[0]),
                    ],
                ),
            ],
            settle: group(
                &fx.settle.0,
                &[(0, b), (3, &pressure[0]), (10, &solid), (2, a)],
            ),
            carry: group(&fx.carry.0, &[(0, a), (2, b)]),
            sharpen: group(&fx.sharpen.0, &[(0, a), (1, b), (10, &solid), (2, c)]),
            drawn: group(&fx.drawn.0, &[(0, a), (9, &drawn_rows)]),
        };
        SmokeWork {
            cells,
            state,
            emitted,
            drawn: field_texture(ctx, cells),
            drawn_rows,
            distances,
            solid,
            passes,
        }
    }

    /// Empties `work`: no speed, no smoke.
    pub fn smoke_clear(&self, ctx: &GpuContext, work: &SmokeWork) {
        ctx.encoder("fx-smoke-clear")
            .clear_buffer(&work.state[0], 0, None);
    }

    /// A copy of where `work` has got to, for later.
    pub fn smoke_keep(&self, ctx: &GpuContext, work: &SmokeWork) -> wgpu::Buffer {
        use wgpu::BufferUsages as U;
        let kept = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-smoke-kept"),
            size: state_bytes(work.cells),
            usage: U::COPY_DST | U::COPY_SRC,
            mapped_at_creation: false,
        });
        ctx.encoder("fx-smoke-keep").copy_buffer_to_buffer(
            &work.state[0],
            0,
            &kept,
            0,
            kept.size(),
        );
        kept
    }

    /// `work` back to a state [`Self::smoke_keep`] kept.
    pub fn smoke_restore(&self, ctx: &GpuContext, work: &SmokeWork, kept: &wgpu::Buffer) {
        let size = kept.size().min(work.state[0].size());
        ctx.encoder("fx-smoke-restore")
            .copy_buffer_to_buffer(kept, 0, &work.state[0], 0, size);
    }

    /// `work`'s state as bytes, for disk or a test. Waits for the card. `None`
    /// when it would not give them back.
    pub fn smoke_read(&self, ctx: &GpuContext, work: &SmokeWork) -> Option<Vec<u8>> {
        use wgpu::BufferUsages as U;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-smoke-read"),
            size: state_bytes(work.cells),
            usage: U::COPY_DST | U::MAP_READ,
            mapped_at_creation: false,
        });
        ctx.encoder("fx-smoke-read").copy_buffer_to_buffer(
            &work.state[0],
            0,
            &staging,
            0,
            staging.size(),
        );
        ctx.flush();
        crate::device::read_buffer(ctx, ("smoke", work.cells[0]), &staging)
    }

    /// `work`'s state from bytes [`Self::smoke_read`] gave. `false` when they
    /// are the wrong size.
    pub fn smoke_write(&self, ctx: &GpuContext, work: &SmokeWork, bytes: &[u8]) -> bool {
        if bytes.len() as u64 != state_bytes(work.cells) {
            return false;
        }
        let from = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-smoke-write"),
                contents: bytes,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        self.smoke_restore(ctx, work, &from);
        true
    }

    /// Lays the particles in `state`, a simulation's state of `cap`, into the
    /// grid `work` emits from this frame. `to_cells` takes their pixels to cells,
    /// by rows.
    pub fn smoke_splat(
        &self,
        ctx: &GpuContext,
        work: &SmokeWork,
        state: &wgpu::Buffer,
        cap: u32,
        to_cells: [[f32; 4]; 3],
    ) {
        let fx = self.smoke_pipes(ctx);
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Splat {
            to_cells: [[f32; 4]; 3],
            cells: [u32; 4],
        }
        let u = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-smoke-splat"),
                contents: bytemuck::bytes_of(&Splat {
                    to_cells,
                    cells: [work.cells[0], work.cells[1], work.cells[2], 0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-smoke-splat"),
            layout: &fx.splat_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: work.emitted.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: u.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-smoke-splat");
        enc.clear_buffer(&work.emitted, 0, None);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-smoke-splat"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&fx.splat);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(cap.div_ceil(64), 1, 1);
    }

    /// Works out which cells `solids` are in this frame, for the steps after. `each`
    /// is a step's numbers.
    pub fn smoke_solids(
        &self,
        ctx: &GpuContext,
        work: &SmokeWork,
        solids: &[SmokeSolid],
        each: &SmokeStep,
    ) {
        let fx = self.smoke_pipes(ctx);
        let Some(steps) = self.step_group(ctx, std::slice::from_ref(each)) else {
            return;
        };
        let list = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-smoke-solids"),
                contents: bytemuck::cast_slice(solids),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-smoke-solids"),
            layout: &fx.solids.0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: work.distances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: list.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: work.solid.as_entire_binding(),
                },
            ],
        });
        let [x, y, z] = work.cells.map(|c| c.div_ceil(4));
        let mut enc = ctx.encoder("fx-smoke-solids");
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-smoke-solids"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&fx.solids.1);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_bind_group(1, &steps, &[0]);
        pass.dispatch_workgroups(x, y, z);
    }

    fn step_group(&self, ctx: &GpuContext, steps: &[SmokeStep]) -> Option<wgpu::BindGroup> {
        let fx = self.smoke_pipes(ctx);
        let mut bytes = vec![0u8; steps.len() * UNIFORM_STEP as usize];
        for (slot, step) in bytes.chunks_exact_mut(UNIFORM_STEP as usize).zip(steps) {
            slot[..std::mem::size_of::<SmokeStep>()].copy_from_slice(bytemuck::bytes_of(step));
        }
        if bytes.is_empty() {
            return None;
        }
        let buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-smoke-steps"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-smoke-steps"),
            layout: &fx.step_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<SmokeStep>() as u64),
                }),
            }],
        }))
    }

    /// `steps`, in order, on `work`.
    ///
    /// The state starts and ends in the first buffer. Between, the third holds
    /// the swirl while the forces read it, the second what the forces leave, the
    /// first that made not to spread, the second that carried along, and the
    /// third that sharpened, which is copied back.
    pub fn smoke_steps(&self, ctx: &GpuContext, work: &SmokeWork, steps: &[SmokeStep]) {
        let fx = self.smoke_pipes(ctx);
        let Some(group) = self.step_group(ctx, steps) else {
            return;
        };
        let [x, y, z] = work.cells.map(|c| c.div_ceil(4));
        let rounds = solve_rounds(work.cells.into_iter().max().unwrap_or(1));
        let mut enc = ctx.encoder("fx-smoke-steps");
        for k in 0..steps.len() {
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fx-smoke-step"),
                    timestamp_writes: None,
                });
                let offset = [k as u32 * UNIFORM_STEP as u32];
                let mut run = |pipeline: &wgpu::ComputePipeline, bind: &wgpu::BindGroup| {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, bind, &[]);
                    pass.set_bind_group(1, &group, &offset);
                    pass.dispatch_workgroups(x, y, z);
                };
                let p = &work.passes;
                run(&fx.curl.1, &p.curl);
                run(&fx.force.1, &p.force);
                run(&fx.spread.1, &p.spread);
                for r in 0..rounds {
                    run(&fx.solve.1, &p.solve[(r % 2) as usize]);
                }
                run(&fx.settle.1, &p.settle);
                run(&fx.carry.1, &p.carry);
                run(&fx.sharpen.1, &p.sharpen);
            }
            enc.copy_buffer_to_buffer(&work.state[2], 0, &work.state[0], 0, work.state[0].size());
        }
    }

    /// Brings [`SmokeWork::fields`] up to where the walk has got to.
    pub fn smoke_draw(&self, ctx: &GpuContext, work: &SmokeWork) {
        let fx = self.smoke_pipes(ctx);
        let mut step: SmokeStep = bytemuck::Zeroable::zeroed();
        step.cells = [
            work.cells[0] as f32,
            work.cells[1] as f32,
            work.cells[2] as f32,
            0.0,
        ];
        let pitch = drawn_pitch(work.cells[0]);
        step.emit_reach[3] = pitch as f32;
        let Some(group) = self.step_group(ctx, &[step]) else {
            return;
        };
        let [x, y, z] = work.cells.map(|c| c.div_ceil(4));
        let mut enc = ctx.encoder("fx-smoke-draw");
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-smoke-draw"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&fx.drawn.1);
            pass.set_bind_group(0, &work.passes.drawn, &[]);
            pass.set_bind_group(1, &group, &[0]);
            pass.dispatch_workgroups(x, y, z);
        }
        enc.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &work.drawn_rows,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch * 4),
                    rows_per_image: Some(work.cells[1]),
                },
            },
            work.drawn.as_image_copy(),
            work.drawn.size(),
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn step(cells: [u32; 3]) -> SmokeStep {
        let c = [cells[0] as f32, cells[1] as f32, cells[2] as f32];
        SmokeStep {
            cells: [c[0], c[1], c[2], 1.0 / 30.0],
            wind: [0.0, 0.0, 0.0, 1.0],
            emit: [4.0, 1.0, 1.0, 0.0],
            emit_middle: [c[0] * 0.5, c[1] * 0.85, c[2] * 0.5, 0.0],
            emit_reach: [c[0] * 0.25, c[1] * 0.1, c[2] * 0.25, 0.0],
            motion: [c[1], 0.0, 20.0, 0.2],
            burn: [2.5, 1.2, 0.5, 1.5],
            fire: [0.3, 10.0, 1.5, 0.0],
            to_model: [[0.0; 4]; 3],
            model_cells: [1.0; 4],
            extra: [0.0; 4],
            more: [3.0, 1.0, 0.0, 0.0],
            drift: [0.0; 4],
        }
    }

    /// Walked twice from empty, or from a state kept part way, a grid comes to
    /// the same bytes, and heat lifts what is emitted off the floor.
    #[test]
    fn the_same_steps_come_to_the_same_bytes_and_heat_rises() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let cells = [24, 32, 24];
        let emit = SmokeEmit {
            distances: None,
            particles: false,
            solids: false,
        };
        let work = fx.smoke_work(&ctx, cells, &emit);
        let s = step(cells);
        let walk = |from: usize, to: usize| {
            for k in from..to {
                let mut at = s;
                at.emit_middle[3] = k as f32 / 30.0;
                fx.smoke_steps(&ctx, &work, &[at]);
            }
        };
        fx.smoke_clear(&ctx, &work);
        walk(0, 12);
        let kept = fx.smoke_keep(&ctx, &work);
        walk(12, 24);
        let first = fx.smoke_read(&ctx, &work).unwrap();
        fx.smoke_clear(&ctx, &work);
        walk(0, 24);
        assert!(first == fx.smoke_read(&ctx, &work).unwrap(), "from empty");
        fx.smoke_restore(&ctx, &work, &kept);
        walk(12, 24);
        assert!(
            first == fx.smoke_read(&ctx, &work).unwrap(),
            "from a kept state"
        );

        // Density by row of cells, top of the grid first.
        let mut rows = vec![0.0f32; cells[1] as usize];
        for z in 0..cells[2] as usize {
            for (y, sum) in rows.iter_mut().enumerate() {
                for x in 0..cells[0] as usize {
                    let at = (x + y * cells[0] as usize + z * (cells[0] * cells[1]) as usize) * 16;
                    let bits = u16::from_le_bytes([first[at + 8], first[at + 9]]);
                    *sum += half::f16::from_bits(bits).to_f32();
                }
            }
        }
        let high: f32 = rows[..cells[1] as usize / 2].iter().sum();
        assert!(high > 1.0, "smoke rose past the middle: {rows:?}");
        assert!(rows.iter().all(|v| v.is_finite()));
    }
}
