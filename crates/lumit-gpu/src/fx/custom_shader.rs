//! The Custom shader's GPU half (docs/impl/custom-shader.md §2.1, §3):
//! validate, compile, cache, and one dispatch.
//!
//! **In plain terms.** Everything about the user's text has already happened by
//! the time this file sees it — `lumit_core::fx::shader` read it, wrapped it in
//! the host's own prologue and epilogue, and handed over one finished module.
//! What is left is the three things that need a graphics card, or nearly:
//!
//! - **Checking it**, with the very shader compiler wgpu is built on, on the
//!   very settings the shipped kernels are held to. No card involved,
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

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// How many compiled pipelines are kept. An eviction is a recompile, never a
/// wrong picture, so the number only has to be larger than the stacks people
/// actually build.
const CACHE_ENTRIES: usize = 32;

/// How many LFX pipelines are kept: one per pass, so enough for eight
/// eight-pass effects on screen at once.
const LFX_CACHE_ENTRIES: usize = 64;

/// How many "last pipeline that compiled" fallbacks are kept, one per effect
/// instance.
///
/// The fallback is what lets a shader keep drawing under a badge while the user
/// is halfway through typing a change into it (§3.2). It only has to cover the
/// instance being edited and its neighbours, so this is generous rather than
/// exact — and it has to exist at all, because an entry pins a whole compiled
/// pipeline and one per instance ever seen is a session-long growth with no
/// ceiling.
const LAST_GOOD_ENTRIES: usize = 32;

/// The compute entry point the assembled module declares (the epilogue's).
const ENTRY_POINT: &str = "lumit_shade";

/// How many pass pictures an LFX layout carries, after the Custom shader's
/// seven bindings. Lumit's own limit on passes.
pub const LFX_PASS_SLOTS: u32 = 8;

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
    ///
    /// Most recently used last, capped at [`LAST_GOOD_ENTRIES`]: each entry
    /// pins a compiled pipeline, and one per instance ever seen would keep
    /// alive exactly what the `cache` LRU above had decided to drop.
    last_good: Mutex<Vec<(u128, Arc<wgpu::ComputePipeline>)>>,
    /// Whether that fallback is allowed at all. **False by default**, which is
    /// the export and headless answer; the interactive realiser turns it on.
    stale_ok: AtomicBool,
    /// How many compiles this engine has run, for the tests that assert one
    /// pipeline per source rather than one per instance.
    compiles: Mutex<u64>,
    /// The LFX effects' layout: the same seven, then a picture per pass.
    lfx_layout: wgpu::BindGroupLayout,
    lfx_pipeline_layout: wgpu::PipelineLayout,
    /// LFX pipelines, most recently used last, keyed by the module's hash.
    lfx_cache: Mutex<Vec<(u64, Arc<wgpu::ComputePipeline>)>>,
    /// LFX modules that would not compile, with the compiler's message, so a
    /// broken shader is checked once rather than every frame.
    lfx_failed: Mutex<Vec<(u64, String)>>,
    /// One transparent pixel, bound for a pass that has not run yet.
    blank: wgpu::Texture,
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
        let mut entries = vec![
            texture(0),
            texture(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: ctx.working(),
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            uniform(3),
            texture(4),
            texture(5),
            uniform(6),
        ];
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-custom-shader-layout"),
                entries: &entries,
            });
        entries.extend((0..LFX_PASS_SLOTS).map(|i| texture(7 + i)));
        let lfx_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-lfx-layout"),
                entries: &entries,
            });
        let pipeline_layout_of = |layout: &wgpu::BindGroupLayout, label: &str| {
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[layout],
                    push_constant_ranges: &[],
                })
        };
        // Its own texture rather than one from the pool, so it is never drawn
        // over and stays the zeros wgpu clears a new texture to.
        let blank = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx-lfx-blank"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ctx.working(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            pipeline_layout: pipeline_layout_of(&layout, "fx-custom-shader-pl"),
            layout,
            cache: Mutex::new(Vec::new()),
            last_good: Mutex::new(Vec::new()),
            stale_ok: AtomicBool::new(false),
            compiles: Mutex::new(0),
            lfx_pipeline_layout: pipeline_layout_of(&lfx_layout, "fx-lfx-pl"),
            lfx_layout,
            lfx_cache: Mutex::new(Vec::new()),
            lfx_failed: Mutex::new(Vec::new()),
            blank,
        }
    }

    /// Forget every compiled shader and count, so an engine handed on
    /// ([`crate::SharedGpu`]) starts the way a new one would.
    pub(super) fn reset(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
        if let Ok(mut last_good) = self.last_good.lock() {
            last_good.clear();
        }
        self.stale_ok
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut compiles) = self.compiles.lock() {
            *compiles = 0;
        }
        if let Ok(mut cache) = self.lfx_cache.lock() {
            cache.clear();
        }
        if let Ok(mut failed) = self.lfx_failed.lock() {
            failed.clear();
        }
    }
}

/// Parse and validate an assembled module the way wgpu will, without a graphics
/// card (the same road the shipped kernels take, on the same settings).
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
            match last.iter().position(|(id, _)| *id == instance) {
                // Move it to the end as well as updating it: the end is the
                // most recently used, which is what makes the eviction below an
                // LRU rather than a coin toss.
                Some(at) => {
                    let mut slot = last.remove(at);
                    slot.1 = pipeline.clone();
                    last.push(slot);
                }
                None => {
                    // Bounded, and it was not.
                    //
                    // One entry per effect instance that has ever compiled,
                    // each pinning a whole compiled pipeline alive — so a long
                    // session, or a project with many Custom shader layers,
                    // grew this without limit and kept every pipeline the
                    // `cache` LRU beside it had already decided to let go of.
                    // The fallback only has to cover the instance a user is
                    // typing into, and its neighbours; a session with more than
                    // [`LAST_GOOD_ENTRIES`] of them has nobody looking at the
                    // oldest.
                    //
                    // An eviction costs a recompile if that instance's source
                    // goes back to being broken, never a wrong picture.
                    if last.len() >= LAST_GOOD_ENTRIES {
                        last.remove(0);
                    }
                    last.push((instance, pipeline.clone()));
                }
            }
        }
    }

    fn compile_shader(
        &self,
        ctx: &GpuContext,
        source_hash: u64,
        assembled: &str,
    ) -> Result<Arc<wgpu::ComputePipeline>, String> {
        let cs = &self.custom_shader;
        self.compile_into(
            ctx,
            source_hash,
            assembled,
            &cs.pipeline_layout,
            &cs.cache,
            CACHE_ENTRIES,
        )
    }

    /// Validate, compile and file one module under `key` in `cache`.
    fn compile_into(
        &self,
        ctx: &GpuContext,
        key: u64,
        assembled: &str,
        layout: &wgpu::PipelineLayout,
        cache: &Mutex<Vec<(u64, Arc<wgpu::ComputePipeline>)>>,
        limit: usize,
    ) -> Result<Arc<wgpu::ComputePipeline>, String> {
        // naga first, always: it is the only road that produces a *message*
        // rather than a device error nobody can read, and it is the same road
        // the shipped kernels take.
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
                layout: Some(layout),
                module: &module,
                entry_point: Some(ENTRY_POINT),
                compilation_options: Default::default(),
                cache: None,
            },
        ));
        if let Ok(mut n) = self.custom_shader.compiles.lock() {
            *n += 1;
        }
        if let Ok(mut cache) = cache.lock() {
            if cache.len() >= limit {
                cache.remove(0);
            }
            cache.push((key, pipeline.clone()));
        }
        Ok(pipeline)
    }

    /// The pipeline for one LFX pass's module, cached by its text's hash.
    /// No stale fallback: an LFX shader is not typed into live.
    ///
    /// # Errors
    /// The compiler's message, when the module does not compile.
    pub fn lfx_pipeline(
        &self,
        ctx: &GpuContext,
        key: u64,
        assembled: &str,
    ) -> Result<Arc<wgpu::ComputePipeline>, String> {
        let cs = &self.custom_shader;
        if let Ok(mut cache) = cs.lfx_cache.lock() {
            if let Some(i) = cache.iter().position(|(h, _)| *h == key) {
                let hit = cache.remove(i);
                let pipeline = hit.1.clone();
                cache.push(hit);
                return Ok(pipeline);
            }
        }
        if let Some((_, why)) = cs
            .lfx_failed
            .lock()
            .ok()
            .and_then(|failed| failed.iter().find(|(k, _)| *k == key).cloned())
        {
            return Err(why);
        }
        let made = self.compile_into(
            ctx,
            key,
            assembled,
            &cs.lfx_pipeline_layout,
            &cs.lfx_cache,
            LFX_CACHE_ENTRIES,
        );
        if let (Err(why), Ok(mut failed)) = (&made, cs.lfx_failed.lock()) {
            if failed.len() >= LFX_CACHE_ENTRIES {
                failed.remove(0);
            }
            failed.push((key, why.clone()));
        }
        made
    }

    /// One dispatch of an LFX pass, writing a `w` by `h` picture. `passes`
    /// holds each pass's picture in pass order, `None` for one that has not
    /// run, which reads as the transparent pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn lfx_pass(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        src: &wgpu::Texture,
        matte: Option<&wgpu::Texture>,
        input2: Option<&wgpu::Texture>,
        passes: &[Option<&wgpu::Texture>],
        w: u32,
        h: u32,
        header: &[u8],
        params: &[u8],
    ) -> wgpu::Texture {
        let cs = &self.custom_shader;
        let slots: Vec<&wgpu::Texture> = (0..LFX_PASS_SLOTS as usize)
            .map(|i| passes.get(i).copied().flatten().unwrap_or(&cs.blank))
            .collect();
        self.dispatch_shader(
            ctx,
            &cs.lfx_layout,
            pipeline,
            [src, src, matte.unwrap_or(src), input2.unwrap_or(src)],
            &slots,
            w,
            h,
            header,
            params,
        )
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
        self.dispatch_shader(
            ctx,
            &self.custom_shader.layout,
            pipeline,
            [src, orig, matte.unwrap_or(src), input2.unwrap_or(src)],
            &[],
            w,
            h,
            header,
            params,
        )
    }

    /// The bind and the dispatch both shader kinds share: `inputs` are
    /// `src`, `orig`, `matte` and `input2`, and `passes` follow binding 6.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_shader(
        &self,
        ctx: &GpuContext,
        layout: &wgpu::BindGroupLayout,
        pipeline: &wgpu::ComputePipeline,
        inputs: [&wgpu::Texture; 4],
        passes: &[&wgpu::Texture],
        w: u32,
        h: u32,
        header: &[u8],
        params: &[u8],
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let [src, orig, matte, input2] = inputs;
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
        // The pictures at 0, 1, 2, 4 and 5, then one per pass from 7.
        let views: Vec<(u32, wgpu::TextureView)> = [0, 1, 2, 4, 5]
            .into_iter()
            .zip([src, orig, &out, matte, input2])
            .chain((7..).zip(passes.iter().copied()))
            .map(|(binding, t)| (binding, t.create_view(&Default::default())))
            .collect();
        let mut entries: Vec<wgpu::BindGroupEntry> = views
            .iter()
            .map(|(binding, v)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: wgpu::BindingResource::TextureView(v),
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: 3,
            resource: head.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: user.as_entire_binding(),
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-custom-shader-bind"),
            layout,
            entries: &entries,
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
