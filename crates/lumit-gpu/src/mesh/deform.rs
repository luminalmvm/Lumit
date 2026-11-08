//! Skins and morph targets on the card: a pass before the mesh pass writes the
//! vertices of every part that moves into one buffer, a pose after a pose, and
//! everything that draws a model reads from there instead of the vertices at
//! rest. A model with neither has nothing here, and one with no pose this frame
//! draws at rest.

use super::{GpuModel, MeshFx, Part, Vertex};
use crate::GpuContext;
use glam::Mat4;
use lumit_scene::pose::Pose;
use lumit_scene::ModelScene;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use wgpu::util::DeviceExt;

/// Where a part's posed vertices are, and whether its skin already put them
/// in model space, so its node doesn't place them again.
#[derive(Clone, Copy)]
pub(super) struct Slot {
    pub(super) out_first: i32,
    pub(super) skinned: bool,
}

/// One part's run of vertices, as `mesh_deform.wgsl` reads it.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Job {
    rest_first: u32,
    count: u32,
    out_first: u32,
    influence_first: u32,
    target_first: u32,
    target_count: u32,
    palette_first: u32,
    weight_first: u32,
    skinned: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Influence {
    joints: [u32; 4],
    weights: [f32; 4],
}

/// What a scene's moving parts need, worked out when it is uploaded.
#[derive(Default)]
pub(super) struct Plan {
    jobs: Vec<Job>,
    slots: HashMap<(usize, usize), Slot>,
    influences: Vec<Influence>,
    targets: Vec<[f32; 4]>,
    /// How many vertices one pose writes.
    total: u32,
}

impl Plan {
    /// `base` finds where each mesh's primitive starts in the vertices at
    /// rest, `None` for one that isn't drawn. A part too large to number is
    /// left at rest.
    pub(super) fn new(scene: &ModelScene, base: impl Fn(usize, usize) -> Option<i32>) -> Self {
        let mut first_joint = Vec::with_capacity(scene.skins.len());
        let mut joints = 0u32;
        for skin in &scene.skins {
            first_joint.push(joints);
            joints += skin.joints.len().min(skin.inverse_bind.len()) as u32;
        }
        let mut plan = Self::default();
        // A primitive two nodes draw shares its numbers.
        let mut influences_at = HashMap::new();
        let mut targets_at = HashMap::new();
        let mut weights = 0u32;
        for (k, node) in scene.nodes.iter().enumerate() {
            let weight_first = weights;
            weights += scene.targets_of(k) as u32;
            let Some(m) = node.mesh else {
                continue;
            };
            let Some(mesh) = scene.meshes.get(m) else {
                continue;
            };
            for (i, p) in mesh.primitives.iter().enumerate() {
                let skin = node.skin.filter(|_| !p.joints.is_empty());
                if skin.is_none() && p.morph_targets.is_empty() {
                    continue;
                }
                let Some(rest_first) = base(m, i).and_then(|b| u32::try_from(b).ok()) else {
                    continue;
                };
                let influence_first = match skin {
                    Some(_) => *influences_at
                        .entry((m, i))
                        .or_insert_with(|| plan.add_influences(p)),
                    None => 0,
                };
                let target_first = *targets_at
                    .entry((m, i))
                    .or_insert_with(|| plan.add_targets(p));
                let out_first = plan.total;
                plan.slots.insert(
                    (k, i),
                    Slot {
                        out_first: out_first as i32,
                        skinned: skin.is_some(),
                    },
                );
                plan.total += p.positions.len() as u32;
                plan.jobs.push(Job {
                    rest_first,
                    count: p.positions.len() as u32,
                    out_first,
                    influence_first,
                    target_first,
                    target_count: p.morph_targets.len() as u32,
                    palette_first: skin.and_then(|s| first_joint.get(s)).copied().unwrap_or(0),
                    weight_first,
                    skinned: u32::from(skin.is_some()),
                });
            }
        }
        plan
    }

    fn add_influences(&mut self, p: &lumit_scene::Primitive) -> u32 {
        let first = self.influences.len() as u32;
        self.influences
            .extend(p.joints.iter().zip(&p.weights).map(|(j, w)| {
                // Shares that don't come to one, or to anything, are the file's
                // slip: scaled to one, or all on the first joint.
                let total: f32 = w.iter().sum();
                let weights = if total > 1e-6 && total.is_finite() {
                    w.map(|s| s / total)
                } else {
                    [1.0, 0.0, 0.0, 0.0]
                };
                Influence {
                    joints: j.map(u32::from),
                    weights,
                }
            }));
        first
    }

    fn add_targets(&mut self, p: &lumit_scene::Primitive) -> u32 {
        let first = self.targets.len() as u32;
        let four = |v: Option<&[f32; 3]>| {
            let v = v.copied().unwrap_or_default();
            [v[0], v[1], v[2], 0.0]
        };
        for t in &p.morph_targets {
            for i in 0..p.positions.len() {
                self.targets.extend([
                    four(t.positions.get(i)),
                    four(t.normals.get(i)),
                    four(t.tangents.get(i)),
                ]);
            }
        }
        first
    }

    /// Where the part drawing node `k`'s primitive `i` lands, if it moves.
    pub(super) fn slot(&self, k: usize, i: usize) -> Option<Slot> {
        self.slots.get(&(k, i)).copied()
    }

    /// Whether any part moves.
    pub(super) fn any(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// The bytes it keeps on the card.
    pub(super) fn bytes(&self) -> u64 {
        std::mem::size_of_val(self.influences.as_slice()) as u64
            + std::mem::size_of_val(self.targets.as_slice()) as u64
    }

    /// Put it on the card. `None` when nothing moves, or when a pose wouldn't
    /// fit in one binding or one dispatch, which leaves the model at rest and
    /// says so ([`left_at_rest`]).
    pub(super) fn upload(
        self,
        ctx: &GpuContext,
        scene: &Arc<ModelScene>,
        rest_bytes: u64,
    ) -> Option<Deforms> {
        if !self.any() {
            return None;
        }
        let limits = ctx.device.limits();
        let largest = u64::from(limits.max_storage_buffer_binding_size);
        let one = u64::from(self.total) * std::mem::size_of::<Vertex>() as u64;
        let most_vertices = u64::from(limits.max_compute_workgroups_per_dimension) * 64;
        let fits = one <= largest
            && rest_bytes <= largest
            && std::mem::size_of_val(self.influences.as_slice()) as u64 <= largest
            && std::mem::size_of_val(self.targets.as_slice()) as u64 <= largest
            && self
                .jobs
                .iter()
                .all(|j| u64::from(j.count) <= most_vertices);
        if !fits {
            if let Ok(mut at_rest) = AT_REST.lock() {
                at_rest.retain(|s| s.strong_count() > 0);
                at_rest.push(Arc::downgrade(scene));
            }
            return None;
        }
        let storage = |label, bytes: &[u8]| {
            // Never an empty buffer, which the card won't make.
            let none = [0u8; 64];
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: if bytes.is_empty() { &none } else { bytes },
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        Some(Deforms {
            influences: storage("mesh-influences", bytemuck::cast_slice(&self.influences)),
            targets: storage("mesh-targets", bytemuck::cast_slice(&self.targets)),
            jobs: self.jobs,
            one,
            posed: Mutex::new(None),
        })
    }
}

/// The scenes too large for this card to move, which draw at rest.
static AT_REST: Mutex<Vec<Weak<ModelScene>>> = Mutex::new(Vec::new());

/// Whether `scene` draws at rest because it is too large for this card to move.
#[must_use]
pub fn left_at_rest(scene: &Arc<ModelScene>) -> bool {
    AT_REST.lock().is_ok_and(|a| {
        a.iter()
            .any(|s| std::ptr::eq(s.as_ptr(), Arc::as_ptr(scene)))
    })
}

/// A model's moving parts on the card.
pub(crate) struct Deforms {
    influences: wgpu::Buffer,
    targets: wgpu::Buffer,
    jobs: Vec<Job>,
    /// The bytes one pose writes.
    one: u64,
    /// The buffer poses are written to, kept from one draw to the next, and
    /// how many it holds.
    posed: Mutex<Option<Written>>,
}

struct Written {
    buffer: wgpu::Buffer,
    stride: u64,
    poses: u64,
    _held: Option<lumit_budget::Reservation>,
}

/// A model in the poses it is drawn in this time, one for each shutter sample
/// or one for the frame. Holds the buffer while it lives, so another draw of
/// the model can't write over it.
pub(super) struct Posed<'a> {
    poses: &'a [Pose],
    model: &'a GpuModel,
    written: Option<Written>,
}

impl MeshFx {
    fn deform_pipeline(
        &self,
        device: &wgpu::Device,
    ) -> &(wgpu::BindGroupLayout, wgpu::ComputePipeline) {
        self.deform.get_or_init(|| {
            let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh-deform"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    storage(1, true),
                    storage(2, true),
                    storage(3, true),
                    storage(4, true),
                    storage(5, true),
                    storage(6, false),
                ],
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mesh-deform"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../mesh_deform.wgsl").into()),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("mesh-deform"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("mesh-deform"),
                        bind_group_layouts: &[&layout],
                        push_constant_ranges: &[],
                    }),
                ),
                module: &module,
                entry_point: Some("cs_deform"),
                compilation_options: Default::default(),
                cache: None,
            });
            (layout, pipeline)
        })
    }

    /// `model` in `poses`, its moving parts written before anything draws
    /// them. With no poses it stays at rest and nothing runs.
    pub(super) fn posed<'a>(
        &self,
        ctx: &GpuContext,
        model: &'a GpuModel,
        poses: &'a [Pose],
    ) -> Posed<'a> {
        let written = model
            .deforms
            .as_ref()
            .filter(|_| !poses.is_empty())
            .and_then(|d| self.write_poses(ctx, model, d, poses));
        Posed {
            poses,
            model,
            written,
        }
    }

    fn write_poses(
        &self,
        ctx: &GpuContext,
        model: &GpuModel,
        d: &Deforms,
        poses: &[Pose],
    ) -> Option<Written> {
        let limits = ctx.device.limits();
        let stride =
            wgpu::util::align_to(d.one, u64::from(limits.min_storage_buffer_offset_alignment));
        let wanted = poses.len() as u64;
        let kept = d.posed.lock().ok().and_then(|mut k| k.take());
        let written = match kept.filter(|k| k.stride == stride && k.poses >= wanted) {
            Some(kept) => kept,
            None => {
                let bytes = stride
                    .checked_mul(wanted)
                    .filter(|&b| b <= limits.max_buffer_size)?;
                let held = ctx.ledger().reserve(lumit_budget::Tier::Vram, bytes).ok();
                if held.is_none() {
                    ctx.charge_vram(bytes);
                }
                Written {
                    buffer: ctx.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("mesh-posed"),
                        size: bytes,
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
                        mapped_at_creation: false,
                    }),
                    stride,
                    poses: wanted,
                    _held: held,
                }
            }
        };
        let (layout, pipeline) = self.deform_pipeline(&ctx.device);
        let palette_len = poses.first().map_or(0, |p| p.palette.len()).max(1);
        let weights_len = poses.first().map_or(0, |p| p.weights.len()).max(1);
        let mut palette = Vec::with_capacity(palette_len * poses.len());
        let mut weights = Vec::with_capacity(weights_len * poses.len());
        // Each pose's run a fixed length, and never empty, which the card
        // won't bind.
        for (k, pose) in poses.iter().enumerate() {
            palette.extend(pose.palette.iter().map(Mat4::to_cols_array_2d));
            palette.resize(palette_len * (k + 1), [[0.0; 4]; 4]);
            weights.extend_from_slice(&pose.weights);
            weights.resize(weights_len * (k + 1), 0.0f32);
        }
        let storage = |label, bytes: &[u8]| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let palette = storage("mesh-palette", bytemuck::cast_slice(&palette));
        let weights = storage("mesh-weights", bytemuck::cast_slice(&weights));
        // A job's numbers for each pose, its palette and weights further along
        // each time.
        let align = u64::from(limits.min_uniform_buffer_offset_alignment);
        let job_stride = wgpu::util::align_to(std::mem::size_of::<Job>() as u64, align);
        let mut uniforms = vec![0u8; (job_stride as usize) * d.jobs.len() * poses.len()];
        for k in 0..poses.len() {
            for (j, job) in d.jobs.iter().enumerate() {
                let job = Job {
                    palette_first: job.palette_first + (k * palette_len) as u32,
                    weight_first: job.weight_first + (k * weights_len) as u32,
                    ..*job
                };
                let at = (k * d.jobs.len() + j) * job_stride as usize;
                uniforms[at..][..std::mem::size_of::<Job>()]
                    .copy_from_slice(bytemuck::bytes_of(&job));
            }
        }
        let uniforms = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-jobs"),
                contents: &uniforms,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let groups: Vec<_> = (0..poses.len() as u64)
            .map(|k| {
                ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mesh-deform"),
                    layout,
                    entries: &[
                        entry(
                            0,
                            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &uniforms,
                                offset: 0,
                                size: wgpu::BufferSize::new(std::mem::size_of::<Job>() as u64),
                            }),
                        ),
                        entry(1, model.vertices.as_entire_binding()),
                        entry(2, d.influences.as_entire_binding()),
                        entry(3, d.targets.as_entire_binding()),
                        entry(4, palette.as_entire_binding()),
                        entry(5, weights.as_entire_binding()),
                        entry(
                            6,
                            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &written.buffer,
                                offset: k * stride,
                                size: wgpu::BufferSize::new(d.one),
                            }),
                        ),
                    ],
                })
            })
            .collect();
        let mut encoder = ctx.encoder("mesh-deform");
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mesh-deform"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            for (k, group) in groups.iter().enumerate() {
                for (j, job) in d.jobs.iter().enumerate() {
                    let at = (k * d.jobs.len() + j) as u64 * job_stride;
                    pass.set_bind_group(0, group, &[at as u32]);
                    pass.dispatch_workgroups(job.count.div_ceil(64), 1, 1);
                }
            }
        }
        Some(written)
    }
}

impl Posed<'_> {
    fn pose(&self, k: usize) -> Option<&Pose> {
        self.poses.get(k).or(self.poses.first())
    }

    /// Where `p`'s node puts it in pose `k`. A skin has put its vertices in
    /// model space already.
    pub(super) fn node(&self, p: &Part, k: usize) -> Mat4 {
        if self.written.is_some() && p.deform.is_some_and(|d| d.skinned) {
            return Mat4::IDENTITY;
        }
        self.pose(k)
            .and_then(|pose| pose.worlds.get(p.index))
            .copied()
            .unwrap_or(p.node)
    }

    /// Whether `p` is hidden in pose `k`.
    pub(super) fn hidden(&self, p: &Part, k: usize) -> bool {
        self.pose(k)
            .and_then(|pose| pose.hidden.get(p.index))
            .copied()
            .unwrap_or(false)
    }

    /// The vertices to draw `p` from in pose `k`, and where its own start.
    pub(super) fn vertices(&self, p: &Part, k: usize) -> (wgpu::BufferSlice<'_>, i32) {
        match (&self.written, p.deform) {
            (Some(w), Some(slot)) => {
                let k = k.min(self.poses.len().saturating_sub(1)) as u64;
                let from = k * w.stride;
                (w.buffer.slice(from..), slot.out_first)
            }
            _ => (self.model.vertices.slice(..), p.base_vertex),
        }
    }

    /// Set `p`'s vertices on `pass` for pose `k`, if they aren't there, and
    /// answer where its own start. `rest` says whether the vertices at rest
    /// are bound, which is all a model at rest ever needs.
    pub(super) fn bind(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        p: &Part,
        k: usize,
        rest: &mut bool,
    ) -> i32 {
        if self.written.is_none() || p.deform.is_none() {
            if !*rest {
                pass.set_vertex_buffer(0, self.model.vertices.slice(..));
                *rest = true;
            }
            return p.base_vertex;
        }
        let (slice, base) = self.vertices(p, k);
        pass.set_vertex_buffer(0, slice);
        *rest = false;
        base
    }
}

impl Drop for Posed<'_> {
    fn drop(&mut self) {
        let (Some(written), Some(d)) = (self.written.take(), self.model.deforms.as_ref()) else {
            return;
        };
        if let Ok(mut kept) = d.posed.lock() {
            *kept = Some(written);
        }
    }
}
