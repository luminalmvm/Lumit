//! The WGSL DIS backend (docs/impl/optical-flow.md §1) — the fast path.
//!
//! In plain terms: the same patch-search algorithm as the CPU oracle in
//! `lib.rs`, but run as GPU compute shaders — thousands of patches solved at
//! once instead of one after another. The shader mirrors the CPU code
//! operation for operation (same loop orders, same constants), which is what
//! lets the tests demand the two agree within float noise. Flow fields live
//! in plain f32 storage buffers rather than fp16 textures for the same
//! reason: fp16 rounding would eat the 1e-3 parity budget (§6.5); textures
//! come back when synthesis itself moves onto the GPU.
//!
//! Failure is always an `Err`, never a fault: callers (`FlowEngine`) degrade
//! to the CPU path.

use crate::{patch_count, FlowField, Gray, PATCH};
use lumit_gpu::GpuContext;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlowError {
    #[error("flow pipeline creation failed: {0}")]
    Pipeline(String),
    #[error("flow readback failed: {0}")]
    Readback(String),
    #[error("frame dimensions differ")]
    DimensionMismatch,
}

/// One uniform block per pyramid level (matches `Params` in dis.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    w: u32,
    h: u32,
    pw: u32,
    ph: u32,
    cw: u32,
    ch: u32,
    npx: u32,
    npy: u32,
    /// Inverse-search iteration cap (Vector detail) — was a shader constant.
    iters: u32,
    /// Smoothing flow-range sigma² (Smoothness) — was a shader constant.
    flow_sigma2: f32,
    pad0: u32,
    pad1: u32,
}

struct Pipelines {
    downsample: wgpu::ComputePipeline,
    sobel: wgpu::ComputePipeline,
    upsample_init: wgpu::ComputePipeline,
    inverse_search: wgpu::ComputePipeline,
    densify: wgpu::ComputePipeline,
    smooth: wgpu::ComputePipeline,
    // Variational refinement (K-332).
    vr_warp: wgpu::ComputePipeline,
    vr_init_duv: wgpu::ComputePipeline,
    vr_deriv: wgpu::ComputePipeline,
    vr_sor_red: wgpu::ComputePipeline,
    vr_sor_black: wgpu::ComputePipeline,
    vr_apply: wgpu::ComputePipeline,
    vr_validity: wgpu::ComputePipeline,
}

/// Per-level GPU resources (both directions share lumas and gradients).
struct Level {
    w: usize,
    h: usize,
    npx: usize,
    npy: usize,
    luma_a: wgpu::Buffer,
    luma_b: wgpu::Buffer,
    grad_a: wgpu::Buffer,
    grad_b: wgpu::Buffer,
    dense_ab: wgpu::Buffer,
    dense_ba: wgpu::Buffer,
    params: wgpu::Buffer,
}

/// One level's prebuilt bind groups for one direction, coarse → fine order.
struct LevelBinds {
    upsample: Option<wgpu::BindGroup>,
    search: wgpu::BindGroup,
    densify: wgpu::BindGroup,
    smooth: wgpu::BindGroup,
    /// The refinement passes (K-332), in dispatch order.
    vr_warp: wgpu::BindGroup,
    vr_init_duv: wgpu::BindGroup,
    vr_deriv: wgpu::BindGroup,
    vr_sor: wgpu::BindGroup,
    vr_apply: wgpu::BindGroup,
    vr_validity: wgpu::BindGroup,
    w: usize,
    h: usize,
    npx: usize,
    npy: usize,
    /// Fixed-point iterations at this level: `refine_iters × (levels − lvl)`,
    /// so finer levels get more, exactly as the oracle does.
    vr_outer: usize,
}

/// Everything prebuilt for one resolution; rebuilt when the size changes.
/// Bind groups live here too — building them per call costs real time at
/// ~70 groups per flow pair.
struct Plan {
    w: usize,
    h: usize,
    /// The settings this plan was built for. A change to any of them reshapes
    /// the pyramid or the uniforms, so the plan is rebuilt rather than reused.
    set: crate::FlowSettings,
    levels: Vec<Level>,
    /// Init-field scratch, shared across levels and directions (sized for
    /// L0; cleared per direction for the coarsest level). The other scratch
    /// buffers — densified-field and patch results — are owned by the bind
    /// groups alone.
    init: wgpu::Buffer,
    staging: wgpu::Buffer,
    /// (bind group, w, h) per downsample dispatch, then per Sobel dispatch.
    pyramid_binds: Vec<(wgpu::BindGroup, usize, usize)>,
    grad_binds: Vec<(wgpu::BindGroup, usize, usize)>,
    /// Per direction (A→B then B→A), per level coarse → fine.
    dir_binds: Vec<Vec<LevelBinds>>,
}

/// The GPU flow solver. Holds its own clones of the device handles (wgpu
/// handles are reference-counted; this shares the one device).
pub struct GpuFlow {
    ctx: GpuContext,
    layout: wgpu::BindGroupLayout,
    pipelines: Pipelines,
    /// Fillers for unused bindings (the layout is shared by all kernels).
    /// Read-only and read-write slots need distinct buffers — a read-write
    /// storage use is exclusive within a dispatch.
    dummy_ro: wgpu::Buffer,
    dummy_rw: wgpu::Buffer,
    plan: Option<Plan>,
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
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

impl GpuFlow {
    /// Build the pipelines on an existing device. Validation problems come
    /// back as `Err`, never a fault.
    pub fn new(ctx: &GpuContext) -> Result<Self, FlowError> {
        let ctx = ctx.clone_handle();
        ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = ctx
            .device
            .create_shader_module(wgpu::include_wgsl!("dis.wgsl"));
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("dis-flow"),
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
                    storage_entry(1, true),
                    storage_entry(2, true),
                    storage_entry(3, true),
                    storage_entry(4, true),
                    storage_entry(5, true),
                    storage_entry(6, false),
                    storage_entry(7, false),
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("dis-flow"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let make = |entry: &str| {
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let pipelines = Pipelines {
            downsample: make("downsample"),
            sobel: make("sobel"),
            upsample_init: make("upsample_init"),
            inverse_search: make("inverse_search"),
            densify: make("densify"),
            smooth: make("smooth_flow"),
            vr_warp: make("vr_warp"),
            vr_init_duv: make("vr_init_duv"),
            vr_deriv: make("vr_deriv"),
            vr_sor_red: make("vr_sor_red"),
            vr_sor_black: make("vr_sor_black"),
            vr_apply: make("vr_apply"),
            vr_validity: make("vr_validity"),
        };
        let mk_dummy = |label: &str| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let dummy_ro = mk_dummy("dis-dummy-ro");
        let dummy_rw = mk_dummy("dis-dummy-rw");
        if let Some(e) = pollster::block_on(ctx.device.pop_error_scope()) {
            return Err(FlowError::Pipeline(e.to_string()));
        }
        Ok(GpuFlow {
            ctx,
            layout,
            pipelines,
            dummy_ro,
            dummy_rw,
            plan: None,
        })
    }

    /// Both flow directions for a luma pair (A→B, B→A), matching the CPU
    /// `flow_pair` bit-closely. Degenerate sizes return the same zeroed
    /// fields the CPU does.
    pub fn flow_pair(&mut self, a: &Gray, b: &Gray) -> Result<(FlowField, FlowField), FlowError> {
        self.flow_pair_with(a, b, &crate::FlowSettings::default())
    }

    /// Both directions under explicit settings.
    ///
    /// Every knob the settings carry is expressed on the GPU: the pyramid
    /// floor shapes the plan, the iteration cap and the smoothing sigma ride
    /// the per-level `Params` uniform, and the refinement count scales by
    /// depth exactly as the CPU's does. The vote-clustering radius stays a
    /// shared constant on both backends deliberately - the CPU's second pass
    /// uses the fixed `FLOW_SIGMA2` there too, so the two agree step for
    /// step. There is no refused configuration; a build error here is a
    /// pipeline fault, not a settings gap.
    pub fn flow_pair_with(
        &mut self,
        a: &Gray,
        b: &Gray,
        set: &crate::FlowSettings,
    ) -> Result<(FlowField, FlowField), FlowError> {
        if a.w != b.w || a.h != b.h {
            return Err(FlowError::DimensionMismatch);
        }
        if a.w < PATCH || a.h < PATCH {
            return Ok((FlowField::zeroed(a.w, a.h), FlowField::zeroed(b.w, b.h)));
        }
        if self
            .plan
            .as_ref()
            .is_none_or(|p| p.w != a.w || p.h != a.h || p.set != *set)
        {
            self.plan = Some(self.build_plan(a.w, a.h, set)?);
        }
        let Some(plan) = self.plan.as_ref() else {
            return Err(FlowError::Pipeline("plan missing".into()));
        };

        // Upload the two full-res lumas; the pyramid is built on the GPU.
        self.ctx
            .queue
            .write_buffer(&plan.levels[0].luma_a, 0, bytemuck::cast_slice(&a.data));
        self.ctx
            .queue
            .write_buffer(&plan.levels[0].luma_b, 0, bytemuck::cast_slice(&b.data));

        let levels = plan.levels.len();
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dis-flow"),
            });
        let wg = |n: usize| n.div_ceil(8) as u32;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dis-pyramid"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines.downsample);
            for (bg, w, h) in &plan.pyramid_binds {
                pass.set_bind_group(0, bg, &[]);
                pass.dispatch_workgroups(wg(*w), wg(*h), 1);
            }
            pass.set_pipeline(&self.pipelines.sobel);
            for (bg, w, h) in &plan.grad_binds {
                pass.set_bind_group(0, bg, &[]);
                pass.dispatch_workgroups(wg(*w), wg(*h), 1);
            }
        }
        // Coarse → fine, each direction. clear_buffer is an encoder-level op,
        // so each direction's coarsest init clear splits the passes.
        let (top_w, top_h) = (plan.levels[levels - 1].w, plan.levels[levels - 1].h);
        for per_level in &plan.dir_binds {
            encoder.clear_buffer(&plan.init, 0, Some((top_w * top_h * 16) as u64));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dis-solve"),
                timestamp_writes: None,
            });
            for lb in per_level {
                if let Some(up) = &lb.upsample {
                    pass.set_pipeline(&self.pipelines.upsample_init);
                    pass.set_bind_group(0, up, &[]);
                    pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                }
                pass.set_pipeline(&self.pipelines.inverse_search);
                pass.set_bind_group(0, &lb.search, &[]);
                pass.dispatch_workgroups(wg(lb.npx), wg(lb.npy), 1);
                pass.set_pipeline(&self.pipelines.densify);
                pass.set_bind_group(0, &lb.densify, &[]);
                pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                pass.set_pipeline(&self.pipelines.smooth);
                pass.set_bind_group(0, &lb.smooth, &[]);
                pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                // DIS part three (K-332). Each fixed-point iteration
                // re-linearises about the current warp, then sweeps the
                // increment red-then-black VR_SOR times — the two colours are
                // what make a sequential solver a parallel one.
                for _ in 0..lb.vr_outer {
                    pass.set_pipeline(&self.pipelines.vr_warp);
                    pass.set_bind_group(0, &lb.vr_warp, &[]);
                    pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                    pass.set_pipeline(&self.pipelines.vr_init_duv);
                    pass.set_bind_group(0, &lb.vr_init_duv, &[]);
                    pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                    pass.set_pipeline(&self.pipelines.vr_deriv);
                    pass.set_bind_group(0, &lb.vr_deriv, &[]);
                    pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                    for _ in 0..crate::VR_SOR {
                        pass.set_pipeline(&self.pipelines.vr_sor_red);
                        pass.set_bind_group(0, &lb.vr_sor, &[]);
                        pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                        pass.set_pipeline(&self.pipelines.vr_sor_black);
                        pass.set_bind_group(0, &lb.vr_sor, &[]);
                        pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                    }
                    pass.set_pipeline(&self.pipelines.vr_apply);
                    pass.set_bind_group(0, &lb.vr_apply, &[]);
                    pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                }
                if lb.vr_outer > 0 {
                    pass.set_pipeline(&self.pipelines.vr_validity);
                    pass.set_bind_group(0, &lb.vr_validity, &[]);
                    pass.dispatch_workgroups(wg(lb.w), wg(lb.h), 1);
                }
            }
        }
        let n = (plan.w * plan.h * 16) as u64;
        encoder.copy_buffer_to_buffer(&plan.levels[0].dense_ab, 0, &plan.staging, 0, n);
        encoder.copy_buffer_to_buffer(&plan.levels[0].dense_ba, 0, &plan.staging, n, n);
        self.ctx.queue.submit([encoder.finish()]);

        // Read both fields back.
        let slice = plan.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| FlowError::Readback(e.to_string()))?
            .map_err(|e| FlowError::Readback(e.to_string()))?;
        let data = slice.get_mapped_range();
        let vals: &[f32] = bytemuck::cast_slice(&data);
        let parse = |offset: usize| {
            let count = plan.w * plan.h;
            let mut f = FlowField::zeroed(plan.w, plan.h);
            for i in 0..count {
                f.u[i] = vals[offset + i * 4];
                f.v[i] = vals[offset + i * 4 + 1];
                f.valid[i] = u8::from(vals[offset + i * 4 + 2] > 0.5);
            }
            f
        };
        let fwd = parse(0);
        let bwd = parse(plan.w * plan.h * 4);
        drop(data);
        plan.staging.unmap();
        Ok((fwd, bwd))
    }

    /// A bind group with `slots` filled and everything else pointing at the
    /// dummy buffer (the layout is shared by all six kernels).
    fn bind(&self, params: &wgpu::Buffer, slots: &[(u32, &wgpu::Buffer)]) -> wgpu::BindGroup {
        let buffer_for = |binding: u32| -> &wgpu::Buffer {
            let dummy = if binding >= 6 {
                &self.dummy_rw
            } else {
                &self.dummy_ro
            };
            slots
                .iter()
                .find(|(b, _)| *b == binding)
                .map_or(dummy, |(_, buf)| buf)
        };
        let entries: Vec<wgpu::BindGroupEntry> = (0..8)
            .map(|binding| wgpu::BindGroupEntry {
                binding,
                resource: if binding == 0 {
                    params.as_entire_binding()
                } else {
                    buffer_for(binding).as_entire_binding()
                },
            })
            .collect();
        self.ctx
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dis-flow"),
                layout: &self.layout,
                entries: &entries,
            })
    }

    fn build_plan(&self, w: usize, h: usize, set: &crate::FlowSettings) -> Result<Plan, FlowError> {
        self.ctx
            .device
            .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        self.ctx
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        // The same level dims the CPU pyramid produces.
        let floor = (set.min_level_dim as usize).max(PATCH);
        let mut dims = vec![(w, h)];
        loop {
            let (lw, lh) = dims[dims.len() - 1];
            let next = ((lw / 2).max(1), (lh / 2).max(1));
            if next.0.min(next.1) < floor {
                break;
            }
            dims.push(next);
        }
        let buf = |label: &str, bytes: usize| {
            self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let mut levels = Vec::with_capacity(dims.len());
        for (l, &(lw, lh)) in dims.iter().enumerate() {
            let (npx, npy) = (patch_count(lw), patch_count(lh));
            let (pw, ph) = if l > 0 { dims[l - 1] } else { (lw, lh) };
            let (cw, ch) = if l + 1 < dims.len() {
                dims[l + 1]
            } else {
                (lw, lh)
            };
            let params = Params {
                w: lw as u32,
                h: lh as u32,
                pw: pw as u32,
                ph: ph as u32,
                cw: cw as u32,
                ch: ch as u32,
                npx: npx as u32,
                npy: npy as u32,
                iters: set.iterations,
                flow_sigma2: set.flow_sigma2(),
                pad0: 0,
                pad1: 0,
            };
            let pbuf = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dis-params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.ctx
                .queue
                .write_buffer(&pbuf, 0, bytemuck::bytes_of(&params));
            levels.push(Level {
                w: lw,
                h: lh,
                npx,
                npy,
                luma_a: buf("dis-luma-a", lw * lh * 4),
                luma_b: buf("dis-luma-b", lw * lh * 4),
                grad_a: buf("dis-grad-a", lw * lh * 16),
                grad_b: buf("dis-grad-b", lw * lh * 16),
                dense_ab: buf("dis-dense-ab", lw * lh * 16),
                dense_ba: buf("dis-dense-ba", lw * lh * 16),
                params: pbuf,
            });
        }
        let np0 = patch_count(w) * patch_count(h);
        let init = buf("dis-init", w * h * 16);
        let tmp = buf("dis-tmp", w * h * 16);
        let patch = buf("dis-patch", np0 * 16);
        // Refinement scratch, sized for L0 and reused at every level and in
        // both directions — a level only ever uses its own w×h prefix, and the
        // levels run one at a time.
        let warp = buf("dis-vr-warp", w * h * 16);
        let warp2 = buf("dis-vr-warp2", w * h * 16);
        let duv = buf("dis-vr-duv", w * h * 16);
        let staging = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dis-staging"),
            size: (w * h * 16 * 2) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        // Prebuild every dispatch's bind group (~70 per resolution).
        let mut pyramid_binds = Vec::new();
        for l in 1..levels.len() {
            let lv = &levels[l];
            pyramid_binds.push((
                self.bind(&lv.params, &[(1, &levels[l - 1].luma_a), (7, &lv.luma_a)]),
                lv.w,
                lv.h,
            ));
            pyramid_binds.push((
                self.bind(&lv.params, &[(1, &levels[l - 1].luma_b), (7, &lv.luma_b)]),
                lv.w,
                lv.h,
            ));
        }
        let mut grad_binds = Vec::new();
        for lv in &levels {
            grad_binds.push((
                self.bind(&lv.params, &[(1, &lv.luma_a), (6, &lv.grad_a)]),
                lv.w,
                lv.h,
            ));
            grad_binds.push((
                self.bind(&lv.params, &[(1, &lv.luma_b), (6, &lv.grad_b)]),
                lv.w,
                lv.h,
            ));
        }
        let mut dir_binds: Vec<Vec<LevelBinds>> = Vec::with_capacity(2);
        for dir_ab in [true, false] {
            let mut per_level = Vec::with_capacity(levels.len());
            for l in (0..levels.len()).rev() {
                let lv = &levels[l];
                let (luma_t, luma_o, grad_t, grad_o) = if dir_ab {
                    (&lv.luma_a, &lv.luma_b, &lv.grad_a, &lv.grad_b)
                } else {
                    (&lv.luma_b, &lv.luma_a, &lv.grad_b, &lv.grad_a)
                };
                let dense = if dir_ab { &lv.dense_ab } else { &lv.dense_ba };
                let upsample = if l + 1 < levels.len() {
                    let coarser = &levels[l + 1];
                    let src = if dir_ab {
                        &coarser.dense_ab
                    } else {
                        &coarser.dense_ba
                    };
                    Some(self.bind(&lv.params, &[(4, src), (6, &init)]))
                } else {
                    None
                };
                per_level.push(LevelBinds {
                    upsample,
                    search: self.bind(
                        &lv.params,
                        &[
                            (1, luma_t),
                            (2, luma_o),
                            (3, grad_t),
                            (4, &init),
                            (6, &patch),
                        ],
                    ),
                    densify: self.bind(
                        &lv.params,
                        &[(1, luma_t), (2, luma_o), (4, &init), (5, &patch), (6, &tmp)],
                    ),
                    smooth: self.bind(&lv.params, &[(1, luma_t), (4, &tmp), (6, dense)]),
                    // Refinement (K-332). `grad_o` is the *other* frame's
                    // Sobel, which is what vr_warp samples along the flow;
                    // every later pass wants the template's instead.
                    vr_warp: self.bind(
                        &lv.params,
                        &[
                            (1, luma_t),
                            (2, luma_o),
                            (3, grad_o),
                            (4, dense),
                            (6, &warp),
                        ],
                    ),
                    vr_init_duv: self.bind(&lv.params, &[(4, dense), (6, &duv)]),
                    vr_deriv: self.bind(&lv.params, &[(4, &warp), (6, &warp2)]),
                    vr_sor: self.bind(
                        &lv.params,
                        &[(3, grad_t), (4, &warp), (5, &warp2), (6, &duv)],
                    ),
                    vr_apply: self.bind(&lv.params, &[(4, &duv), (6, dense)]),
                    vr_validity: self.bind(
                        &lv.params,
                        &[(1, luma_t), (2, luma_o), (3, grad_t), (6, dense)],
                    ),
                    w: lv.w,
                    h: lv.h,
                    npx: lv.npx,
                    npy: lv.npy,
                    // The oracle scales the iteration count by depth from the
                    // coarsest level; `per_level` is built coarse-first, so
                    // that is simply how many levels have been pushed already.
                    vr_outer: set.refine_iters as usize * (per_level.len() + 1),
                });
            }
            dir_binds.push(per_level);
        }
        let plan = Plan {
            w,
            h,
            set: *set,
            levels,
            init,
            staging,
            pyramid_binds,
            grad_binds,
            dir_binds,
        };
        for _ in 0..2 {
            if let Some(e) = pollster::block_on(self.ctx.device.pop_error_scope()) {
                return Err(FlowError::Pipeline(e.to_string()));
            }
        }
        Ok(plan)
    }
}
