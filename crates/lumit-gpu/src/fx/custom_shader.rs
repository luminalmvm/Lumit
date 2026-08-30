//! The Custom shader's GPU half (docs/impl/custom-shader.md §2.1, §3, K-650):
//! validate, compile, cache, and one dispatch.
//!
//! **In plain terms.** Everything about the user's text has already happened by
//! the time this file sees it — `lumit_core::fx::shader` read it, wrapped it in
//! the host's own prologue and epilogue, and handed over one finished module.
//! What is left is the three things that need a graphics card, or nearly:
//!
//! - **Checking it**, with the very shader compiler wgpu is built on, on the
//!   very settings the shipped kernels are held to (K-263). No card involved,
//!   which is what lets a machine with no graphics hardware tell a person their
//!   shader is broken; and running it here rather than at pipeline creation is
//!   what turns a black frame on a stranger's adapter into a sentence.
//! - **Keeping it.** One compiled pipeline per distinct source, not per instance:
//!   two layers running the same shader compile once and share it. What they do
//!   **not** share is the buffer of numbers their controls hold — getting that
//!   backwards in either direction is a plausible bug with a very confusing
//!   symptom, so the pipeline is cached here and the uniform is built per
//!   dispatch.
//! - **Falling back, carefully.** While somebody is typing, their source does not
//!   compile most of the time, and going black on every keystroke is punishment
//!   UI. So the last pipeline that *did* compile can keep drawing under a badge —
//!   but only where a person is looking. Export and headless never fall back:
//!   [`FxEngine::allow_stale_shaders`] is off unless the interactive realiser
//!   turns it on, and a frame drawn from a stale pipeline is never cached.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::{GpuContext, WORKING_FORMAT};

use super::{work_texture, FxEngine};

/// How many compiled pipelines are kept. An eviction is a recompile, never a
/// wrong picture, so the number only has to be larger than the stacks people
/// actually build.
const CACHE_ENTRIES: usize = 32;

/// The compute entry point the assembled module declares (the epilogue's).
const ENTRY_POINT: &str = "lumit_shade";

/// The Custom shader's own bind group layout and its compiled pipelines.
///
/// Seven entries rather than the shared fx five, so it is its own layout exactly
/// as the LUT's is: the two extra pictures and the user's own uniform have
/// nowhere to go on the shared one.
pub struct CustomShaderPipelines {
    pub(super) layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    /// Compiled pipelines, most recently used last, keyed by the source hash.
    cache: Mutex<Vec<(u64, Arc<wgpu::ComputePipeline>)>>,
    /// The last pipeline that compiled, per effect instance — what an
    /// interactive render falls back to while the source will not compile.
    last_good: Mutex<Vec<(u128, Arc<wgpu::ComputePipeline>)>>,
    /// Whether that fallback is allowed at all. **False by default**, which is
    /// the export and headless answer; the interactive realiser turns it on.
    stale_ok: AtomicBool,
    /// How many compiles this engine has run, for the tests that assert one
    /// pipeline per source rather than one per instance.
    compiles: Mutex<u64>,
}

impl CustomShaderPipelines {
    pub(super) fn new(ctx: &GpuContext) -> Self {
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
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-custom-shader-layout"),
                entries: &[
                    texture(0),
                    texture(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    uniform(3),
                    texture(4),
                    texture(5),
                    uniform(6),
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-custom-shader-pl"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        Self {
            layout,
            pipeline_layout,
            cache: Mutex::new(Vec::new()),
            last_good: Mutex::new(Vec::new()),
            stale_ok: AtomicBool::new(false),
            compiles: Mutex::new(0),
        }
    }
}

/// Parse and validate an assembled module the way wgpu will, without a graphics
/// card (K-263's road, on K-263's settings).
///
/// `Capabilities::empty()` is the load-bearing argument: it is what the shipped
/// kernels are held to, and a custom shader that asked for more would compile on
/// its author's machine and be a black frame on somebody else's.
///
/// # Errors
/// naga's own message, verbatim — somebody else's sentence about somebody else's
/// code, which is why it is not translated. The caller remaps its line numbers to
/// the user's own text
/// (`lumit_core::fx::shader::ShaderProgram::remap_error`).
pub fn validate(assembled: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(assembled)
        .map_err(|e| e.emit_to_string(assembled))
        .map_err(|m| m.trim_end().to_owned())?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    validator
        .validate(&module)
        .map_err(|e| e.emit_to_string(assembled).trim_end().to_owned())?;
    Ok(())
}

impl FxEngine {
    /// Whether a broken source may keep drawing with this instance's last good
    /// pipeline (§3.2). Off unless an **interactive** render turns it on: an
    /// export that silently used yesterday's shader would be worse than one that
    /// says the shader is broken.
    pub fn allow_stale_shaders(&self, on: bool) {
        self.custom_shader.stale_ok.store(on, Ordering::Relaxed);
    }

    /// How many custom-shader pipelines this engine has compiled — one per
    /// distinct source, whatever the stack does with them.
    pub fn shader_compiles(&self) -> u64 {
        self.custom_shader.compiles.lock().map_or(0, |n| *n)
    }

    /// The pipeline for one instance's source: the cached one, a freshly
    /// compiled one, or — interactively, and only there — the last one that
    /// compiled, with the compiler's message beside it.
    ///
    /// `Ok((pipeline, None))` is the shader the document says. `Ok((pipeline,
    /// Some(message)))` is the stale affordance: a picture with a badge over it,
    /// which the caller must show and discard rather than file under any name.
    ///
    /// # Errors
    /// The compiler's message, when the source does not compile and there is
    /// nothing to fall back to (or falling back is not allowed here).
    #[allow(clippy::type_complexity)]
    pub fn shader_pipeline(
        &self,
        ctx: &GpuContext,
        instance: u128,
        source_hash: u64,
        assembled: &str,
    ) -> Result<(Arc<wgpu::ComputePipeline>, Option<String>), String> {
        let cs = &self.custom_shader;
        // Looked up, and released, before anything touches the device: a lock
        // held across a GPU call is the one docs/14 §5 names.
        if let Ok(mut cache) = cs.cache.lock() {
            if let Some(i) = cache.iter().position(|(h, _)| *h == source_hash) {
                let hit = cache.remove(i);
                let pipeline = hit.1.clone();
                cache.push(hit);
                self.remember_good(instance, &pipeline);
                return Ok((pipeline, None));
            }
        }
        match self.compile_shader(ctx, source_hash, assembled) {
            Ok(pipeline) => {
                self.remember_good(instance, &pipeline);
                Ok((pipeline, None))
            }
            Err(why) => {
                if cs.stale_ok.load(Ordering::Relaxed) {
                    if let Some(stale) = cs.last_good.lock().ok().and_then(|m| {
                        m.iter()
                            .find(|(id, _)| *id == instance)
                            .map(|(_, p)| p.clone())
                    }) {
                        return Ok((stale, Some(why)));
                    }
                }
                Err(why)
            }
        }
    }

    fn remember_good(&self, instance: u128, pipeline: &Arc<wgpu::ComputePipeline>) {
        if let Ok(mut last) = self.custom_shader.last_good.lock() {
            match last.iter_mut().find(|(id, _)| *id == instance) {
                Some(slot) => slot.1 = pipeline.clone(),
                None => last.push((instance, pipeline.clone())),
            }
        }
    }

    fn compile_shader(
        &self,
        ctx: &GpuContext,
        source_hash: u64,
        assembled: &str,
    ) -> Result<Arc<wgpu::ComputePipeline>, String> {
        // naga first, always: it is the only road that produces a *message*
        // rather than a device error nobody can read, and it is the same road
        // K-263 holds the shipped kernels to.
        validate(assembled)?;
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fx-custom-shader"),
                source: wgpu::ShaderSource::Wgsl(assembled.into()),
            });
        let pipeline = Arc::new(ctx.device.create_compute_pipeline(
            &wgpu::ComputePipelineDescriptor {
                label: Some("fx-custom-shader"),
                layout: Some(&self.custom_shader.pipeline_layout),
                module: &module,
                entry_point: Some(ENTRY_POINT),
                compilation_options: Default::default(),
                cache: None,
            },
        ));
        if let Ok(mut n) = self.custom_shader.compiles.lock() {
            *n += 1;
        }
        if let Ok(mut cache) = self.custom_shader.cache.lock() {
            if cache.len() >= CACHE_ENTRIES {
                cache.remove(0);
            }
            cache.push((source_hash, pipeline.clone()));
        }
        Ok(pipeline)
    }

    /// One dispatch of a compiled custom shader.
    ///
    /// `header` is `lumit_core::fx::shader::ShaderHeader`'s bytes and `params`
    /// the buffer that module's own `Params` struct describes — both built by the
    /// caller, because both are arithmetic over the document rather than over the
    /// device. `matte` and `input2` stand in as `src` when nothing is bound,
    /// since a texture binding cannot be left empty; the header's `matte_on` and
    /// `input2_on` are what say whether they mean anything.
    #[allow(clippy::too_many_arguments)]
    pub fn custom_shader(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        src: &wgpu::Texture,
        orig: &wgpu::Texture,
        matte: Option<&wgpu::Texture>,
        input2: Option<&wgpu::Texture>,
        w: u32,
        h: u32,
        header: &[u8],
        params: &[u8],
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-custom-shader-out");
        let head = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-custom-shader-header"),
                contents: header,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let user = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-custom-shader-params"),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-custom-shader-bind"),
            layout: &self.custom_shader.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(orig)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: head.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&view(matte.unwrap_or(src))),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&view(input2.unwrap_or(src))),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: user.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-custom-shader-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-custom-shader-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
