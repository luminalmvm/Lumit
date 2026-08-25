//! GPU frame synthesis (docs/impl/optical-flow.md §3) — the other half of the
//! fast path.
//!
//! In plain terms: `gpu.rs` works out *how everything moved*; this takes that
//! answer and paints the in-between frame with it. Both endpoints get dragged
//! along their motion to where they would be at the requested moment, then
//! blended, with pixels that only exist in one of the two frames taken from
//! that one alone.
//!
//! It is deliberately not held to the bit-parity `gpu.rs` is. docs/08 §3.1 pins
//! the contract as "vector-field tolerance, then bit-tolerant synthesis": the
//! field is the measurement, and this is a resample of it. That latitude is
//! what lets the flow stay at its working resolution here rather than being
//! upsampled to frame size first — which on the CPU path was most of the cost,
//! and bought nothing, since a bilinear read of a bilinearly-upsampled field is
//! a bilinear read of the field.

use crate::{FlowField, FlowSettings};
use lumit_gpu::GpuContext;

use crate::gpu::FlowError;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SynthParams {
    w: u32,
    h: u32,
    fw: u32,
    fh: u32,
    phi: f32,
    occ_mode: u32,
    fallback: u32,
    hud_on: u32,
}

/// Buffers for one frame/field size, kept between calls.
///
/// A 1080p synthesis allocates about 25 MB across seven buffers; doing that per
/// frame costs more than the compute does. Sizes only change when the source or
/// the flow resolution does, so the set is rebuilt then and reused otherwise.
struct Buffers {
    w: usize,
    h: usize,
    fw: usize,
    fh: usize,
    params: wgpu::Buffer,
    fa: wgpu::Buffer,
    fb: wgpu::Buffer,
    bf: wgpu::Buffer,
    bb: wgpu::Buffer,
    out: wgpu::Buffer,
    staging: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// The synthesis pipelines, built once per device.
pub struct GpuSynth {
    ctx: GpuContext,
    layout: wgpu::BindGroupLayout,
    prep: wgpu::ComputePipeline,
    hud: wgpu::ComputePipeline,
    post: wgpu::ComputePipeline,
    blend: wgpu::ComputePipeline,
    buffers: std::cell::RefCell<Option<Buffers>>,
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

impl GpuSynth {
    pub fn new(ctx: &GpuContext) -> Result<Self, FlowError> {
        let ctx = GpuContext::from_parts(ctx.device.clone(), ctx.queue.clone());
        ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = ctx
            .device
            .create_shader_module(wgpu::include_wgsl!("synth.wgsl"));
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("flow-synth"),
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
                    storage_entry(5, false),
                    storage_entry(6, false),
                    storage_entry(7, false),
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flow-synth"),
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
        let prep = make("syn_prep");
        let hud = make("syn_hud");
        let post = make("syn_post");
        let blend = make("syn_blend");
        if let Some(e) = pollster::block_on(ctx.device.pop_error_scope()) {
            return Err(FlowError::Pipeline(e.to_string()));
        }
        Ok(GpuSynth {
            ctx,
            layout,
            prep,
            hud,
            post,
            blend,
            buffers: std::cell::RefCell::new(None),
        })
    }

    /// Synthesise the frame at `phi` between RGBA8 frames `a` and `b`, from
    /// flow fields measured at their own (possibly reduced) resolution.
    ///
    /// Errors never fault — the caller falls back to the CPU oracle.
    #[allow(clippy::too_many_arguments)]
    pub fn synthesize(
        &self,
        a: &[u8],
        b: &[u8],
        w: usize,
        h: usize,
        fwd: &FlowField,
        bwd: &FlowField,
        phi: f32,
        set: &FlowSettings,
    ) -> Result<Vec<u8>, FlowError> {
        let n = w * h;
        if a.len() < n * 4 || b.len() != a.len() || fwd.w != bwd.w || fwd.h != bwd.h {
            return Err(FlowError::DimensionMismatch);
        }
        let fw = fwd.w;
        let fh = fwd.h;
        let fn_ = fw * fh;
        if fn_ == 0 {
            return Err(FlowError::DimensionMismatch);
        }
        let dev = &self.ctx.device;
        // Rebuild the buffer set only when a size changes.
        {
            let mut slot = self.buffers.borrow_mut();
            let stale = slot
                .as_ref()
                .is_none_or(|b| b.w != w || b.h != h || b.fw != fw || b.fh != fh);
            if stale {
                let storage = |label: &str, bytes: usize| {
                    dev.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size: bytes.max(16) as u64,
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::COPY_DST
                            | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    })
                };
                let params = dev.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("synth-params"),
                    size: std::mem::size_of::<SynthParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let fa = storage("synth-a", n * 4);
                let fb = storage("synth-b", n * 4);
                let bf = storage("synth-fwd", fn_ * 16);
                let bb = storage("synth-bwd", fn_ * 16);
                let aux = storage("synth-aux", fn_ * 16);
                let aux2 = storage("synth-aux2", fn_ * 16);
                let out = storage("synth-out", n * 4);
                let staging = dev.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("synth-staging"),
                    size: (n * 4) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                fn entry(binding: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
                    wgpu::BindGroupEntry {
                        binding,
                        resource: buf.as_entire_binding(),
                    }
                }
                let bind = dev.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("flow-synth"),
                    layout: &self.layout,
                    entries: &[
                        entry(0, &params),
                        entry(1, &fa),
                        entry(2, &fb),
                        entry(3, &bf),
                        entry(4, &bb),
                        entry(5, &aux),
                        entry(6, &aux2),
                        entry(7, &out),
                    ],
                });
                *slot = Some(Buffers {
                    w,
                    h,
                    fw,
                    fh,
                    params,
                    fa,
                    fb,
                    bf,
                    bb,
                    out,
                    staging,
                    bind,
                });
            }
        }
        let slot = self.buffers.borrow();
        let Some(bufs) = slot.as_ref() else {
            return Err(FlowError::Pipeline("synth buffers missing".into()));
        };

        let params = SynthParams {
            w: w as u32,
            h: h as u32,
            fw: fw as u32,
            fh: fh as u32,
            phi,
            occ_mode: match set.occlusion {
                crate::OcclusionMode::VisibleOnly => 0,
                crate::OcclusionMode::Blend => 1,
            },
            fallback: match set.fallback {
                crate::Fallback::Blend => 0,
                crate::Fallback::Nearest => 1,
            },
            hud_on: u32::from(set.hud_guard),
        };
        let q = &self.ctx.queue;
        q.write_buffer(&bufs.params, 0, bytemuck::bytes_of(&params));
        q.write_buffer(&bufs.fa, 0, &a[..n * 4]);
        q.write_buffer(&bufs.fb, 0, &b[..n * 4]);
        // (u, v, valid, 0) per pixel — the same vec4 layout dis.wgsl emits.
        let pack_field = |f: &FlowField| -> Vec<f32> {
            let mut v = vec![0f32; fn_ * 4];
            for i in 0..fn_ {
                v[i * 4] = f.u[i];
                v[i * 4 + 1] = f.v[i];
                v[i * 4 + 2] = f32::from(f.valid[i]);
            }
            v
        };
        q.write_buffer(&bufs.bf, 0, bytemuck::cast_slice(&pack_field(fwd)));
        q.write_buffer(&bufs.bb, 0, bytemuck::cast_slice(&pack_field(bwd)));

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("flow-synth"),
        });
        let wg = |v: usize| v.div_ceil(8) as u32;
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flow-synth"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &bufs.bind, &[]);
            pass.set_pipeline(&self.prep);
            pass.dispatch_workgroups(wg(fw), wg(fh), 1);
            pass.set_pipeline(&self.hud);
            pass.dispatch_workgroups(wg(fw), wg(fh), 1);
            pass.set_pipeline(&self.post);
            pass.dispatch_workgroups(wg(fw), wg(fh), 1);
            pass.set_pipeline(&self.blend);
            pass.dispatch_workgroups(wg(w), wg(h), 1);
        }
        enc.copy_buffer_to_buffer(&bufs.out, 0, &bufs.staging, 0, (n * 4) as u64);
        self.ctx.queue.submit([enc.finish()]);

        let slice = bufs.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| FlowError::Readback(e.to_string()))?
            .map_err(|e| FlowError::Readback(e.to_string()))?;
        let data = slice.get_mapped_range();
        let pixels = data.to_vec();
        drop(data);
        bufs.staging.unmap();
        Ok(pixels)
    }
}
