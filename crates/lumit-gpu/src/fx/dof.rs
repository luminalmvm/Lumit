//! The passes that carry their own bespoke bind-group layout rather than the
//! shared two-input one: depth-of-field lens blur (docs/08 DoF foundation),
//! the 3-D LUT lookup (docs/08 §3.11) and the adjustment-layer blend.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// The most aperture blades the depth-of-field polygon test carries. Bounds the
/// kernel's per-tap loop and the uniform's normal array.
///
/// Declared here rather than imported: `lumit-core` is only a dev-dependency of
/// this crate (the kernels take plain numbers and know nothing of the document
/// model), so `lumit_core::fx::MAX_BLADES` is out of reach in production code.
/// `max_blades_matches_the_core_constant` in `fx::tests` — where lumit-core IS
/// available — pins the two together so they cannot drift.
pub const MAX_BLADES: usize = 8;

/// One resolved depth-of-field pass (docs/08 §3.22). The depth pass arrives as
/// its own texture (see [`upload_depth_map`] and [`FxEngine::dof`]); everything
/// else the kernel needs is here.
///
/// Field for field this is `lumit_core::fx::cpu::DofParams`, which is what lets
/// the §1.6 oracle set both paths up from one value, and makes a field added to
/// one side an obvious omission on the other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DofOp {
    /// The in-focus depth, 0..1, when `use_focus_point` is clear.
    pub focus: f32,
    /// Half-width of the sharp band around focus, 0..1.
    pub range: f32,
    /// Near-side max CoC radius (depths in front of focus), raster px.
    pub near_aperture: f32,
    /// Far-side max CoC radius (depths behind focus), raster px — and the one
    /// uniform radius when no depth is bound.
    pub far_aperture: f32,
    /// Outward unit edge normals, the first `blade_count` live. Computed by the
    /// caller — the kernel calls no trig, so the oracle reproduces it exactly.
    pub blade_normals: [[f32; 2]; MAX_BLADES],
    /// 3..=[`MAX_BLADES`]. Inert while `roundness` is 1: a circle has no blades.
    pub blade_count: u32,
    /// `cos²(π/N)`.
    pub apothem2: f32,
    /// −1 star … 0 polygon … 1 circle. 1 is the default and takes the plain
    /// `r² ≤ coc²` test — the aperture this effect has always gathered.
    pub roundness: f32,
    /// −1 centre-weighted … 0 flat disc … 1 rim-weighted.
    pub rim: f32,
    /// Tap-offset multipliers, both ≥ 1 and exactly one > 1, so the aperture can
    /// only shrink on one axis and never reaches outside the circle.
    pub aspect_scale: [f32; 2],
    /// The tonal split level and the power its excess is raised to. A power of
    /// exactly 1 is the plain arithmetic mean and skips the split.
    pub threshold: f32,
    pub bokeh_power: f32,
    /// Clamp the gather to the frame edge instead of pulling in transparency.
    pub repeat_edge: bool,
    /// False = no depth layer: the whole frame defocuses at `far_aperture` and
    /// `depth` is never sampled, so the caller may bind any same-size texture.
    pub depth_bound: bool,
    /// Which channel of `depth` is read, by `lumit_core::fx::CHANNEL_OPTIONS`.
    pub depth_channel: u32,
    pub depth_invert: bool,
    /// When set, focus is the depth under `focus_point` and `focus` is ignored —
    /// the greyed row in the panel.
    pub use_focus_point: bool,
    /// Raster px.
    pub focus_point: [f32; 2],
    /// Multiplier on the depth distance before the ramp (the Profile control,
    /// resolved). 1 is the plain full-range falloff.
    pub gamma: f32,
    pub remove_edge_leak: f32,
    pub detect_edge_threshold: f32,
    /// Diagnostic view: 0 = Rendered, 1 = Depth map, 2 = Focus map.
    pub display: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The `dof` kernel's uniform. Layout mirrors `fx_dof.wgsl`'s `Params` field for
/// field: seventeen floats, ten `u32`s and one pad word — 28 words, seven whole
/// 16-byte rows — then the normals as an `array<vec4<f32>, 8>`. 240 bytes.
///
/// **That word count is load-bearing.** An `array<vec4<f32>, N>` is 16-byte
/// aligned in WGSL, so if the scalars above stop being a multiple of four the
/// shader moves `blade_normals` to the next multiple of 16 while `repr(C)` moves
/// it to the next multiple of 4, and every normal is then read from the wrong
/// offset. Adding one scalar above without restoring the count is exactly how
/// that happens; it costs no arithmetic and fails loudly in the §1.6 oracle
/// (measured at 17 920 fp16 ULP when it did).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DofParams {
    focus: f32,
    range: f32,
    near_aperture: f32,
    far_aperture: f32,
    mix_amt: f32,
    apothem2: f32,
    roundness: f32,
    rim: f32,
    aspect_x: f32,
    aspect_y: f32,
    threshold: f32,
    bokeh_power: f32,
    focus_x: f32,
    focus_y: f32,
    gamma: f32,
    remove_edge_leak: f32,
    detect_edge_threshold: f32,
    /// 0 = read the depth as-is, 1 = invert it (`d' = 1 - d`) before the CoC.
    depth_invert: u32,
    /// Diagnostic view: 0 = Rendered, 1 = Depth map, 2 = Focus map.
    display: u32,
    blade_count: u32,
    depth_bound: u32,
    depth_channel: u32,
    use_focus_point: u32,
    repeat_edge: u32,
    /// Whether the gather weights its taps at all, whether it splits them at the
    /// threshold, and whether the aperture is the plain circle. All three are
    /// decided host-side and once, because none of the neutral settings is an
    /// IEEE identity: `Σ(c·w)/Σw` is not `Σc/n` when every `w` is 1,
    /// `min(c,t) + max(c−t,0)` is not reliably `c`, and scaling both sides of a
    /// comparison by `apothem2` can flip a boundary tap. The neutral settings
    /// must take a genuinely different path, not multiply by one.
    weighted: u32,
    tonal: u32,
    circle: u32,
    /// Padding to a whole 16-byte row — see the type docs; the array below is
    /// 16-byte aligned in WGSL and only 4-byte aligned under `repr(C)`.
    _pad0: u32,
    /// Only `.xy` of each element is read.
    blade_normals: [[f32; 4]; MAX_BLADES],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AdjustParams {
    opacity: f32,
    _pad: [f32; 3],
}

/// The generic Matte dissolve's one number (K-395): 1 to invert the matte.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MatteMixParams {
    invert: f32,
    _pad: [f32; 3],
}

/// The matte preparation's two numbers (K-425): the channel and the invert.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MattePrepareParams {
    channel: u32,
    invert: u32,
    _pad: [u32; 2],
}

/// The effect Blend's two numbers (K-425): the mode and the effect's Mix.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlendMixParams {
    mode: u32,
    mix: f32,
    _pad: [f32; 2],
}

/// One resolved 3D-LUT lookup (docs/08 §3.11; docs/impl/lut.md). The cube
/// itself arrives as its own 3D texture (see [`upload_lut_3d`] and
/// [`FxEngine::lut`]); this uniform carries the edge length the shader needs to
/// turn a colour into grid coordinates, the host Mix, and the cube's input
/// domain (K-271 — the shader remaps through it exactly as the CPU reference
/// does; before that it assumed 0..1 and a cube saying otherwise rendered
/// silently wrong).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LutParams {
    /// LUT edge length N (the cube holds `N³` samples).
    size: u32,
    /// 0..1, blended against the unprocessed input.
    mix: f32,
    /// The Input space the lookup happens in (K-443): 0 Linear, 1 sRGB,
    /// 2 Rec. 709 — `lumit_core::lut::LutSpace::code`'s numbering, which the
    /// shader's `to_space` / `to_linear` branch on. 0 is the identity both ways.
    space: u32,
    _pad: f32,
    /// `DOMAIN_MIN`, per channel; the fourth lane is padding (a uniform vec3
    /// is 16-byte aligned regardless, so it costs nothing).
    domain_min: [f32; 4],
    /// `DOMAIN_MAX`, per channel, same padding.
    domain_max: [f32; 4],
}

impl FxEngine {
    /// Apply one depth-of-field lens blur to a linear working texture,
    /// returning a new texture of the same size. Backs the `dof` effect
    /// (docs/08 §3.22, docs/impl/layer-input.md).
    ///
    /// One pass. Per output pixel it reads the depth from the channel `op` names
    /// (Red by convention and by default; `textureLoad`, not a sampler),
    /// optionally inverts it (`d' = 1 - d`, swapping near and far), turns it
    /// into a circle-of-confusion radius — zero inside `range` of the focus
    /// depth, ramping smoothstep (scaled first by `gamma`, the Profile
    /// control) to `near_aperture` raster pixels on the near side or
    /// `far_aperture` on the far side — then averages an aperture of that radius
    /// from `src` and blends against the input by the host Mix.
    ///
    /// The focus depth is `op.focus`, or the depth under `op.focus_point` when
    /// `use_focus_point` is set. With `depth_bound` clear the depth is never
    /// sampled and the whole frame defocuses at `far_aperture`, so the caller may
    /// bind any same-size float texture in that slot.
    ///
    /// **The aperture and the average are both shapeable, and every shaping
    /// control is branched around at its neutral** (K-313): the aperture's
    /// **Roundness** reaches below zero into star shapes and **Deform** squeezes
    /// it on one axis — both leave it inscribed in the circle of the CoC radius,
    /// so `ceil(radius)` stays a correct bound on the taps and the effect's ROI
    /// stays honest — while **Concentration** weights the taps radially and
    /// **Remove edge leak** pulls back taps sitting across a depth
    /// discontinuity. At Roundness 1, Concentration 0, edge leak 0 and Exposure
    /// 0 the kernel takes the plain circle test and the plain unweighted,
    /// unsplit sum, which is the box-weighted disc average this pass has always
    /// computed — bit for bit, which is what the passthrough tests pin.
    ///
    /// `display` selects the output view: 0 = Rendered (the blur above),
    /// 1 = Depth map (the post-invert, post-channel-pick depth as greyscale),
    /// 2 = Focus map (the smooth `1 - s` in-focus mask); the diagnostic views
    /// ignore the blur and Mix and are continuous, so the oracle covers them.
    ///
    /// `depth` must be the same size as `src`; because it is read through
    /// `textureLoad` rather than a sampler, it may be **any float texture** —
    /// the referenced depth layer rendered in the working `rgba16float` format
    /// (the effect's real depth input), or the exact R32Float map the §1.6
    /// oracle uploads. `depth` is consumed exactly as `lumit_core::fx::cpu::dof`
    /// (the CPU oracle) reads it and the tap set is identical, so the two agree.
    /// Shares [`Self::mb_layout`] with Motion blur — the depth field is the one
    /// extra sampled input over the two-input convention. Both apertures zero,
    /// a depth everywhere inside the sharp band, or a Mix of 0 are bit-exact
    /// passthroughs.
    pub fn dof(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        depth: &wgpu::Texture,
        op: &DofOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-dof-out");
        let mut blade_normals = [[0.0f32; 4]; MAX_BLADES];
        for (dst, n) in blade_normals.iter_mut().zip(op.blade_normals.iter()) {
            dst[0] = n[0];
            dst[1] = n[1];
        }
        // Decided here, once, rather than per tap: see `DofParams::weighted`.
        let weighted = op.rim != 0.0 || (op.remove_edge_leak > 0.0 && op.depth_bound);
        let tonal = op.bokeh_power != 1.0;
        let circle = op.roundness >= 1.0 && op.aspect_scale == [1.0, 1.0];
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-dof-params"),
                contents: bytemuck::bytes_of(&DofParams {
                    focus: op.focus,
                    range: op.range,
                    near_aperture: op.near_aperture,
                    far_aperture: op.far_aperture,
                    mix_amt: op.mix,
                    apothem2: op.apothem2,
                    roundness: op.roundness,
                    rim: op.rim,
                    aspect_x: op.aspect_scale[0],
                    aspect_y: op.aspect_scale[1],
                    threshold: op.threshold,
                    bokeh_power: op.bokeh_power,
                    focus_x: op.focus_point[0],
                    focus_y: op.focus_point[1],
                    gamma: op.gamma,
                    remove_edge_leak: op.remove_edge_leak,
                    detect_edge_threshold: op.detect_edge_threshold,
                    depth_invert: u32::from(op.depth_invert),
                    display: op.display,
                    blade_count: op.blade_count,
                    depth_bound: u32::from(op.depth_bound),
                    depth_channel: op.depth_channel,
                    use_focus_point: u32::from(op.use_focus_point),
                    repeat_edge: u32::from(op.repeat_edge),
                    weighted: u32::from(weighted),
                    tonal: u32::from(tonal),
                    circle: u32::from(circle),
                    _pad0: 0,
                    blade_normals,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-dof-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                // orig-for-mix: a single pass, so the unprocessed original is
                // the source itself.
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(depth)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
                // The Matte slot Motion blur added to this shared layout
                // (K-429). This kernel never reads it, so `src` stands in it:
                // a binding cannot be left empty, and it is the same
                // "bound but not read" convention `dispatch_matted` uses.
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-dof-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-dof-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.dof);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// Apply one 3D-LUT lookup (docs/08 §3.11; docs/impl/lut.md) to a linear
    /// working texture, returning a new texture of the same size. One pass on
    /// **unpremultiplied** colour (§2.2 — a LUT is an arbitrary colour map):
    /// per output pixel, unpremultiply, convert into the Input space `space`
    /// names (K-443 — 0 Linear, 1 sRGB, 2 Rec. 709; Linear is the identity and
    /// the bit-exact picture this pass rendered before), map each channel through
    /// `[domain_min, domain_max]` to a grid coordinate in `[0, size-1]`
    /// (clamped, and a zero span reading as 0), `textureLoad` the eight
    /// integer corners of `lut_tex` and trilinearly interpolate in f32 — **not**
    /// the hardware sampler, whose precision is not guaranteed bit-for-bit
    /// across GPUs (docs/impl/lut.md §3) — convert back to scene-linear,
    /// re-premultiply, then blend against
    /// the input by the host Mix. The cube is consumed exactly as
    /// `lumit_core::lut::Lut3d::sample_in` reads its red-fastest data, so the two
    /// agree (§1.6). Its own bind group (the cube is a 3D texture, the one
    /// binding no other kernel has). `mix == 0` is the bit-exact input.
    #[allow(clippy::too_many_arguments)]
    pub fn lut(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        lut_tex: &wgpu::Texture,
        size: u32,
        mix: f32,
        space: u32,
        domain_min: [f32; 3],
        domain_max: [f32; 3],
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-lut-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lut-params"),
                contents: bytemuck::bytes_of(&LutParams {
                    size,
                    mix,
                    space,
                    _pad: 0.0,
                    domain_min: [domain_min[0], domain_min[1], domain_min[2], 0.0],
                    domain_max: [domain_max[0], domain_max[1], domain_max[2], 0.0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        // The cube is a 3D texture; name its view dimension explicitly so the
        // binding matches the layout's `D3` regardless of the default.
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-lut-bind"),
            layout: &self.lut_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                // orig-for-mix: a single pass, so the unprocessed original is
                // the source itself.
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-lut-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-lut-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.lut);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// The adjustment-layer blend (docs/06 §1.5): per-channel lerp between
    /// the accumulated composite `below` and its effected copy `processed`,
    /// by `coverage`'s alpha (the layer's comp-space mask raster) times
    /// `opacity` (the layer opacity, 0..1). All three textures are comp
    /// sized; returns a new comp-sized working texture.
    #[allow(clippy::too_many_arguments)]
    pub fn adjust_blend(
        &self,
        ctx: &GpuContext,
        below: &wgpu::Texture,
        processed: &wgpu::Texture,
        coverage: &wgpu::Texture,
        w: u32,
        h: u32,
        opacity: f32,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-adjust-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-adjust-params"),
                contents: bytemuck::bytes_of(&AdjustParams {
                    opacity,
                    _pad: [0.0; 3],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-adjust-bind"),
            layout: &self.adjust_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &below.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &processed.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &coverage.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &out.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-adjust-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-adjust-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.adjust);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// The generic Matte dissolve (K-395, docs/08 §2.6): per-channel lerp from
    /// the picture the effect was given (`input`) to what it produced
    /// (`processed`), by the `matte`'s premultiplied Rec. 709 luma — inverted
    /// when `invert`. The op-for-op twin of
    /// [`lumit_core::fx::cpu::matte_mix`](../../../lumit_core/fx/cpu/fn.matte_mix.html);
    /// all three textures are this raster's size, and a new one comes back.
    ///
    /// It is never called when no matte is bound, which is what makes an effect
    /// with an unset Matte row byte-identical to the same effect before K-395
    /// (K-258): the pass does not run, so there is nothing to be identical to.
    #[allow(clippy::too_many_arguments)]
    pub fn matte_mix(
        &self,
        ctx: &GpuContext,
        input: &wgpu::Texture,
        processed: &wgpu::Texture,
        matte: &wgpu::Texture,
        w: u32,
        h: u32,
        invert: bool,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-matte-mix-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-matte-mix-params"),
                contents: bytemuck::bytes_of(&MatteMixParams {
                    invert: f32::from(invert),
                    _pad: [0.0; 3],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-matte-mix-bind"),
            layout: &self.adjust_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &input.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &processed.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &matte.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &out.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-matte-mix-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-matte-mix-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.matte_mix);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// One pass on the adjustment layout — three sampled inputs, a storage
    /// output and a uniform — the shape `matte_prepare` and `blend_mix` share
    /// with `matte_mix`.
    #[allow(clippy::too_many_arguments)]
    fn three_input_pass(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        label: &str,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        c: &wgpu::Texture,
        w: u32,
        h: u32,
        params: &[u8],
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, label);
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.adjust_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &a.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &b.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &c.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &out.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder(label);
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// The matte's Channel pick and Invert, once (K-425, docs/08 §2.6): the
    /// RGBA `matte` becomes a grey picture whose R = G = B = the chosen
    /// channel (a `CHANNEL_OPTIONS` index), clamped and inverted if asked,
    /// alpha 1. The op-for-op twin of
    /// [`lumit_core::fx::cpu::matte_prepare`](../../../lumit_core/fx/cpu/fn.matte_prepare.html).
    ///
    /// The seam calls it only when
    /// [`matte_needs_prepare`](lumit_core::fx::cpu::matte_needs_prepare) says
    /// so: Luminance with Invert off is what every kernel reads already, and
    /// not running the pass is what keeps that case byte for byte (K-258).
    pub fn matte_prepare(
        &self,
        ctx: &GpuContext,
        matte: &wgpu::Texture,
        w: u32,
        h: u32,
        channel: u32,
        invert: bool,
    ) -> wgpu::Texture {
        self.three_input_pass(
            ctx,
            &self.matte_prepare,
            "fx-matte-prepare",
            matte,
            matte,
            matte,
            w,
            h,
            bytemuck::bytes_of(&MattePrepareParams {
                channel,
                invert: u32::from(invert),
                _pad: [0; 2],
            }),
        )
    }

    /// The effect Blend and Mix, once (K-425, docs/08 §1.5): `processed` is
    /// the kernel's output at Mix 100, `input` what it was given; each pixel
    /// becomes `input * (1 - mix) + blend(input, processed) * mix` for `mode`
    /// a `BlendMode::ALL` index. The op-for-op twin of
    /// [`lumit_core::fx::cpu::blend_mix`](../../../lumit_core/fx/cpu/fn.blend_mix.html).
    /// Never called for Normal, which runs no pass at all.
    #[allow(clippy::too_many_arguments)]
    pub fn blend_mix(
        &self,
        ctx: &GpuContext,
        input: &wgpu::Texture,
        processed: &wgpu::Texture,
        w: u32,
        h: u32,
        mode: u32,
        mix: f32,
    ) -> wgpu::Texture {
        self.three_input_pass(
            ctx,
            &self.blend_mix,
            "fx-blend-mix",
            input,
            processed,
            processed,
            w,
            h,
            bytemuck::bytes_of(&BlendMixParams {
                mode,
                mix,
                _pad: [0.0; 2],
            }),
        )
    }
}
