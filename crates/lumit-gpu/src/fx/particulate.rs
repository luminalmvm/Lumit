//! Particulate's GPU passes (docs/08 §3.86, K-446, K-474, K-475;
//! docs/impl/particulate.md §5–§7): evaluate, compact, draw.
//!
//! **In plain terms.** The kernel next door works out where every particle is
//! and packs the live ones together; this file is the host half — it makes the
//! buffers, hands the numbers over, and runs the four passes in order. Nothing
//! here does arithmetic on a parameter: every value arrives already reduced by
//! `lumit_core::fx::effects::particulate`'s own `packed`, so the CPU reference
//! and this path multiply by numbers that came from one expression.
//!
//! **Why it is four passes and not one.** A particle system's output is
//! *variable length* — how many are alive is not known until they have all been
//! asked — and a GPU cannot append to a list without either atomics or a
//! prefix sum. Atomics would make the order of the compacted stream depend on
//! which workgroup arrived first, which would make `id` order a scheduling
//! artefact and two renders of one frame able to disagree. So: count, scan,
//! place. Deterministic by construction (particulate.md §5).

use crate::{GpuContext, WORKING_FORMAT};

use super::{work_texture, FxEngine};

/// The most births one frame's window may record before the oldest are let go.
///
/// The candidate set is Emit rate times the longest life, and both of those are
/// open-ended controls — a rate of a million and a life of an hour is a typeable
/// document, and a buffer sized off it is an allocation with no ceiling
/// (14-ENGINEERING-RULES §6). Eight times the hard cap is thirty-two megabytes
/// of flags, and dropping candidates *older* than that changes nothing the cap
/// rule would not already have dropped: what survives an overload is the newest
/// `cap`, and there are eight times that many newer candidates still in play.
pub const MAX_CANDIDATES: u64 = 8_000_000;

/// One resolved Particulate. Mirrors `lumit_core::fx::points::PointsParams`,
/// `DrawStyle` and the threaded schedule, flattened to the numbers the kernel
/// reads — every distance in **raster** pixels, as the resolve step left them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticulateOp<'a> {
    pub emitter_pos: [f32; 2],
    /// The Line/Ellipse/Rectangle extents.
    pub emitter_wh: [f32; 2],
    pub emitter_angle_deg: f32,
    pub direction_deg: f32,
    pub spread_deg: f32,
    pub speed: f32,
    /// `0..=1`.
    pub speed_jitter: f32,
    /// The `EmitterShape` code: 0 Point, 1 Line, 2 Ellipse, 3 Rectangle,
    /// 4 Mask path.
    pub shape: u32,
    pub seed: u32,
    /// The most particles to **draw**: Max particles, halved once per
    /// degradation rung (K-475).
    pub cap: u32,
    pub life: f32,
    /// `0..=1`.
    pub life_jitter: f32,
    pub size: f32,
    /// `0..=1`.
    pub size_jitter: f32,
    pub rotation_deg: f32,
    pub rotation_jitter_deg: f32,
    pub spin_deg: f32,
    pub align_to_motion: bool,
    /// Scene-linear RGBA at birth and at death.
    pub colour: [f32; 4],
    pub end_colour: [f32; 4],
    pub wind: [f32; 2],
    pub gravity: f32,
    pub drag: f32,
    pub turbulence: f32,
    pub turbulence_scale: f32,
    pub turbulence_speed: f32,
    /// The **fixed** central-difference step the turbulence speed is read at,
    /// so one frame key names one picture whatever raster is previewing.
    pub eps: f32,
    /// The comp frame's length in seconds of layer time.
    pub dt: f32,
    /// The birth index of candidate zero.
    pub first_birth: u64,
    /// One entry per recorded frame plus a closing one: `[birth offset,
    /// bit pattern of (t − that frame's start) in seconds]`. The subtraction is
    /// the host's, in `f64`, so the small age never has to be recovered from
    /// inside a large clock.
    pub frames: &'a [[u32; 2]],
    /// How many births the window records — the candidate count.
    pub candidates: u32,
    /// The two baked over-life curves, size then opacity, 257 entries each.
    pub curves: &'a [f32],
    /// The mask path as `(x, y, arc length, unused)` per vertex (K-408); empty
    /// for every other emitter shape, and the documented no-op for Mask path.
    pub path: &'a [[f32; 4]],
    pub path_total: f32,
    /// Streak length in seconds; zero in every other mode.
    pub tail_seconds: f32,
    /// `0..=1`.
    pub feather: f32,
    /// The host Mix, `0..=1`, folded into the particle's own coverage (K-425).
    pub mix: f32,
    /// The `RenderMode` code: 0 Disc, 1 Sprite, 2 Streak. **An unset Sprite
    /// arrives as Disc** — the host resolves the fallback so the kernel has one
    /// less branch (particulate.md §2).
    pub mode: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticulateParams {
    em_pos: [f32; 2],
    em_wh: [f32; 2],
    em_angle: f32,
    dir: f32,
    spread: f32,
    speed: f32,

    speed_jitter: f32,
    shape: u32,
    seed: u32,
    cap: u32,

    life: f32,
    life_jitter: f32,
    size: f32,
    size_jitter: f32,

    rotation: f32,
    rotation_jitter: f32,
    spin: f32,
    align: u32,

    colour: [f32; 4],
    end_colour: [f32; 4],

    wind: [f32; 2],
    gravity: f32,
    drag: f32,

    turb: f32,
    turb_scale: f32,
    turb_speed: f32,
    eps: f32,

    dt: f32,
    first_birth_lo: u32,
    first_birth_hi: u32,
    frames: u32,

    candidates: u32,
    path_len: u32,
    path_total: f32,
    tail: f32,

    feather: f32,
    mix: f32,
    mode: u32,
    target_w: f32,

    target_h: f32,
    sprite_w: f32,
    sprite_h: f32,
    _pad: f32,
}

/// The scan's block width — the same 256 the kernel declares.
const SCAN_BLOCK: u32 = 256;

/// Words of stream per particle: position 2, speed 2, age, life, size,
/// rotation, colour 2 (half pairs), id 2, and the draw's own tail 2.
const STREAM_WORDS: u64 = 14;

impl FxEngine {
    /// Draw one Particulate over a working texture, returning a new texture of
    /// the same size (docs/08 §3.86).
    ///
    /// `sprite` is the Sprite layer's rendered picture (K-123), `None` in every
    /// other mode **and** when the row is unset — in which case `op.mode` is
    /// already Disc, because a render mode must always draw something.
    ///
    /// The picture is copied first and the particles drawn over it, so this is
    /// an ordinary "picture in, picture out" pass with an instanced draw in the
    /// middle rather than a compositor of its own.
    pub fn particulate(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        sprite: Option<&wgpu::Texture>,
        op: &ParticulateOp<'_>,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-particulate-out");
        {
            // The input, unchanged, is what a frame with no live particle looks
            // like — so the copy happens first and every early return below is
            // the effect's honest passthrough.
            let mut enc = ctx.encoder("fx-particulate-copy");
            enc.copy_texture_to_texture(
                src.as_image_copy(),
                out.as_image_copy(),
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let Some(pass) = self.particulate_prepare(ctx, src, w, h, sprite, op) else {
            return out;
        };
        let view = out.create_view(&Default::default());
        let mut enc = ctx.encoder("fx-particulate");
        pass.evaluate(self, &mut enc);
        // **The cancellation point** (particulate.md §7) is here, between the
        // evaluate and the draw: two passes, one boundary. No token reaches an
        // effect kernel today — the export's flag is checked once a frame
        // (`export::run`) — so the seam is named rather than faked, and a
        // cancel lands on the frame this pass belongs to.
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx-particulate-draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The picture is already there, copied in above.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rp.set_pipeline(&self.particulate_draw);
            rp.set_bind_group(0, &pass.empty, &[]);
            rp.set_bind_group(1, &pass.draw_bind, &[]);
            rp.draw_indirect(&pass.args, 0);
        }
        drop(enc);
        out
    }

    /// The compacted stream itself, read back to the host — the **oracle hook**
    /// (K-019, docs/08 §1.6), and what PS7's goldens will pin.
    ///
    /// Runs the evaluate and compaction passes and nothing else, then maps the
    /// stream buffer. It has `readback_linear_f32`'s shape and its reason: a
    /// test that can only see pixels can only hold the *picture* to the CPU
    /// reference, and points-stream.md §3.2 asks a stricter question of the
    /// numbers — ≤ 2 ULP, because a consumer reads them as data and data has no
    /// perceptual tolerance.
    ///
    /// Answers the live count and the stream's words; `None` when the device
    /// would not give them back.
    pub fn particulate_stream(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ParticulateOp<'_>,
    ) -> Option<(u32, Vec<u32>)> {
        let pass = self.particulate_prepare(ctx, src, w, h, None, op)?;
        let stream_bytes = u64::from(op.cap) * STREAM_WORDS * 4;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-readback"),
            size: stream_bytes + 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        {
            let mut enc = ctx.encoder("fx-particulate-eval");
            pass.evaluate(self, &mut enc);
            enc.copy_buffer_to_buffer(&pass.args, 0, &staging, 0, 16);
            enc.copy_buffer_to_buffer(&pass.stream, 0, &staging, 16, stream_bytes);
        }
        ctx.flush();
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv().ok()?.ok()?;
        let words: Vec<u32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        staging.unmap();
        // Word 1 of the indirect arguments is the instance count: how many
        // particles the cap rule left standing.
        let count = *words.get(1)?;
        Some((count, words.get(4..)?.to_vec()))
    }

    /// Everything one Particulate pass needs bound, or `None` when there is
    /// nothing to draw — no candidates, no capacity, or a Mix of nothing, each
    /// of which leaves the input picture exactly as it was.
    fn particulate_prepare(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        sprite: Option<&wgpu::Texture>,
        op: &ParticulateOp<'_>,
    ) -> Option<ParticulatePass> {
        use wgpu::util::DeviceExt;
        let candidates = op.candidates;
        let cap = op.cap;
        if candidates == 0 || cap == 0 || op.mix <= 0.0 || op.frames.len() < 2 {
            return None;
        }
        let blocks = candidates.div_ceil(SCAN_BLOCK);
        let sprite_size = sprite.map_or((1.0, 1.0), |t| (t.width() as f32, t.height() as f32));
        let u = ParticulateParams {
            em_pos: op.emitter_pos,
            em_wh: op.emitter_wh,
            em_angle: op.emitter_angle_deg,
            dir: op.direction_deg,
            spread: op.spread_deg,
            speed: op.speed,
            speed_jitter: op.speed_jitter,
            shape: op.shape,
            seed: op.seed,
            cap,
            life: op.life,
            life_jitter: op.life_jitter,
            size: op.size,
            size_jitter: op.size_jitter,
            rotation: op.rotation_deg,
            rotation_jitter: op.rotation_jitter_deg,
            spin: op.spin_deg,
            align: u32::from(op.align_to_motion),
            colour: op.colour,
            end_colour: op.end_colour,
            wind: op.wind,
            gravity: op.gravity,
            drag: op.drag,
            turb: op.turbulence,
            turb_scale: op.turbulence_scale,
            turb_speed: op.turbulence_speed,
            eps: op.eps,
            dt: op.dt,
            first_birth_lo: op.first_birth as u32,
            first_birth_hi: (op.first_birth >> 32) as u32,
            frames: op.frames.len() as u32 - 1,
            candidates,
            path_len: op.path.len() as u32,
            path_total: op.path_total,
            tail: op.tail_seconds,
            feather: op.feather,
            mix: op.mix,
            mode: op.mode,
            target_w: w as f32,
            target_h: h as f32,
            sprite_w: sprite_size.0,
            sprite_h: sprite_size.1,
            _pad: 0.0,
        };
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-params"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let frames_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-frames"),
                contents: bytemuck::cast_slice(op.frames),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // A storage binding cannot be empty, so an absent path is one dead
        // vertex nothing reads — `path_len` under two is the no-op the kernel
        // checks, never the buffer's length.
        let path_data: &[[f32; 4]] = if op.path.is_empty() {
            &[[0.0; 4]]
        } else {
            op.path
        };
        let path_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-path"),
                contents: bytemuck::cast_slice(path_data),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let curves_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-particulate-curves"),
                contents: bytemuck::cast_slice(op.curves),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let scratch = |label: &str, words: u64| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: words.max(1) * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let ranks = scratch("fx-particulate-ranks", u64::from(candidates));
        let sums = scratch("fx-particulate-blocks", u64::from(blocks) + 1);
        // **The declared peak scratch** (docs/13 §6): the cap *is* the budget,
        // which is why Max particles is a parameter and not a guess.
        let stream = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-stream"),
            size: u64::from(cap) * STREAM_WORDS * 4,
            // COPY_SRC so the §1.6 oracle can read the stream back
            // (`particulate_stream`). It costs an allocation flag and buys the
            // only test that can hold these numbers to two ULP.
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let args = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-particulate-args"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-bind"),
            layout: &self.particulate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frames_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: curves_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: path_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ranks.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: stream.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: args.as_entire_binding(),
                },
            ],
        });
        let draw_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-draw-bind"),
            layout: &self.particulate_draw_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: stream.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &sprite.unwrap_or(src).create_view(&Default::default()),
                    ),
                },
            ],
        });
        let empty = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-particulate-empty"),
            layout: &self.particulate_empty_layout,
            entries: &[],
        });
        Some(ParticulatePass {
            bind,
            draw_bind,
            empty,
            stream,
            args,
            candidates,
            blocks,
            // The buffers the bind group points at, held so they outlive it.
            _keep: [ubuf, frames_buf, curves_buf, path_buf, ranks, sums],
        })
    }
}

/// One pass's bound resources, alive until the encoder is done with them.
struct ParticulatePass {
    bind: wgpu::BindGroup,
    draw_bind: wgpu::BindGroup,
    empty: wgpu::BindGroup,
    stream: wgpu::Buffer,
    args: wgpu::Buffer,
    candidates: u32,
    blocks: u32,
    _keep: [wgpu::Buffer; 6],
}

impl ParticulatePass {
    /// Count, scan, place — the three kernels, in the one order that makes a
    /// compacted slot a function of the birth index rather than of which
    /// workgroup arrived first.
    fn evaluate(&self, fx: &FxEngine, enc: &mut wgpu::CommandEncoder) {
        let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx-particulate-evaluate"),
            timestamp_writes: None,
        });
        cp.set_bind_group(0, &self.bind, &[]);
        cp.set_pipeline(&fx.particulate_alive);
        cp.dispatch_workgroups(self.candidates.div_ceil(64), 1, 1);
        cp.set_pipeline(&fx.particulate_scan);
        cp.dispatch_workgroups(self.blocks, 1, 1);
        cp.set_pipeline(&fx.particulate_blocks);
        cp.dispatch_workgroups(1, 1, 1);
        cp.set_pipeline(&fx.particulate_scatter);
        cp.dispatch_workgroups(self.candidates.div_ceil(64), 1, 1);
    }
}

/// The five pipelines and three layouts, built once per device.
pub(super) struct ParticulatePipelines {
    pub layout: wgpu::BindGroupLayout,
    pub draw_layout: wgpu::BindGroupLayout,
    pub empty_layout: wgpu::BindGroupLayout,
    pub alive: wgpu::ComputePipeline,
    pub scan: wgpu::ComputePipeline,
    pub blocks: wgpu::ComputePipeline,
    pub scatter: wgpu::ComputePipeline,
    pub draw: wgpu::RenderPipeline,
}

impl ParticulatePipelines {
    pub(super) fn new(ctx: &GpuContext, module: &wgpu::ShaderModule) -> Self {
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    storage(1, true),
                    storage(2, true),
                    storage(3, true),
                    storage(4, false),
                    storage(5, false),
                    storage(6, false),
                    storage(7, false),
                ],
            });
        // The draw's own group. The stream is bound **read-only** here: a
        // vertex stage may not write, and it has nothing to write anyway.
        let draw_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-draw-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        // The draw's bindings live at `@group(1)` because the compute half
        // already owns `@group(0)` in the same module, and one module is what
        // keeps the kernel and its twin in one file.
        let empty_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-particulate-empty-layout"),
                entries: &[],
            });
        let compute_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-pl"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let draw_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-particulate-draw-pl"),
                bind_group_layouts: &[&empty_layout, &draw_layout],
                push_constant_ranges: &[],
            });
        let compute = |entry: &str, label: &str| {
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: Some(&compute_pl),
                    module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let draw = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("fx-particulate-draw"),
                layout: Some(&draw_pl),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("pt_vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("pt_fs"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: WORKING_FORMAT,
                        // Premultiplied `over`, the same blend the compositor's
                        // Normal uses: the particle's own colour, and what was
                        // there kept by however much it did not cover.
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                // **Never multisampled.** An fp16 blend into a multisample
                // target is where the physical flare's nondeterminism came
                // from, and a particle field is that same shape at scale.
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });
        Self {
            layout,
            draw_layout,
            empty_layout,
            alive: compute("pt_alive", "fx-particulate-alive"),
            scan: compute("pt_scan", "fx-particulate-scan"),
            blocks: compute("pt_blocks", "fx-particulate-blocks"),
            scatter: compute("pt_scatter", "fx-particulate-scatter"),
            draw,
        }
    }
}
