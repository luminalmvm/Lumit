//! Time-based kernels (docs/08 §3.2, §3.12, §3.13): echo/trails, flow motion
//! blur and datamosh, each sampling neighbour frames or a flow field.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved echo (docs/08 §3.13; blend modes + 16-echo cap since
/// FX-17). The neighbour frames arrive as textures keyed by offset;
/// `weights[i]` is the tap intensity for the echo at offset `-(i+1)`
/// (0 = skip). `mode` is the combine blend: 0 = Add, 1 = Behind, 2 = Max,
/// 3 = Screen, 4 = Normal, 5 = Multiply, 6 = Overlay, 7 = Soft light,
/// 8 = Hard light, 9 = Darken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoOp {
    pub weights: [f32; 16],
    pub mode: u32,
    /// 0..1, blended against the leading (current) frame.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EchoParams {
    weight: f32,
    mode: u32,
    /// 1 = the matte scales Decay per pixel.
    matte_on: f32,
    /// Which tap this dispatch folds in (0 = one frame back): the exponent
    /// the per-pixel decay is raised to.
    tap: i32,
}

/// Side of the square tiles the dominant-motion reduction works in, in pixels
/// (docs/impl/optical-flow.md §4.5 item 3).
///
/// Duplicated from `lumit_core::fx::cpu::MB_TILE` rather than imported, because
/// this crate deliberately does not depend on lumit-core outside its tests
/// (docs/05 §1.1). `wgsl_motion_blur_matches_the_cpu_oracle` asserts the two are
/// equal, so the duplication cannot drift silently.
pub const MB_TILE: u32 = 16;

/// One resolved flow motion blur (docs/08 §3.2). The per-pixel motion is a
/// dense flow field passed as its own texture (see [`upload_flow_field`] and
/// [`FxEngine::motion_blur`]); this op carries only the scalars the kernel
/// turns a vector into a streak with. `samples` must equal the tap *cap*
/// `lumit_core::fx::effects::motion_blur::MotionBlur::packed` produces, so the
/// GPU adapts its tap count inside the CPU oracle's exact bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionBlurOp {
    /// Shutter ÷ 360: streak length as a fraction of the inter-frame motion.
    pub shutter_frac: f32,
    /// The cap on bilinear taps along the streak; the count adapts below it
    /// (docs/impl/optical-flow.md §4).
    pub samples: i32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// Output view (FX-19): 0 Rendered, 1 Motion vectors, 2 Confidence,
    /// 3 Dominant motion — the `lumit_core::fx::MbView::code()` integer, so the
    /// kernel matches the CPU oracle's `view` branch.
    pub view: i32,
    /// Reconstruction tier: the `lumit_core::fx::MbQuality::code()` integer
    /// (0 Normal, 1 High — curved trails and half the tap spacing).
    pub quality: i32,
    /// px@raster a full **Motion vectors** channel means. Read only
    /// when a vectors layer is bound; the measured flow is already in pixels.
    pub vector_scale: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionBlurParams {
    shutter_frac: f32,
    samples: i32,
    mix_amt: f32,
    view: i32,
    tile: i32,
    quality: i32,
    /// 1 = the matte scales Shutter angle per pixel.
    matte_on: f32,
    _pad: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MbTileParams {
    tile: i32,
    /// px@raster a full Motion vectors channel means, read by `mb_vectors`
    /// alone; the reduction ignores it.
    vector_scale: f32,
    _pad: [i32; 2],
}

/// One resolved Datamosh pass (docs/08 §3.12; its own effect, reworked to a
/// flow-driven melt by T19). The raw -1 source neighbour and the dense
/// current→previous flow field arrive as their own textures (see
/// [`FxEngine::datamosh`]); this op carries the melt scalars.
/// Callers fold the schema's Intensity and host Mix into `intensity` before
/// calling (mixing the same two inputs twice collapses to one mix by the
/// product), so this kernel and its CPU oracle need no second blend knob.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DatamoshOp {
    /// Blended over the current frame; > 1 extrapolates (FX-14).
    pub intensity: f32,
    /// Frames of predicted motion the streamline walk reaches; the
    /// walk's `steps` taps span it.
    pub displacement: f32,
    /// 0..1, how much of the reach accumulates into the smear: 0 a
    /// short trail, 1 a long melting bloom.
    pub bloom: f32,
    /// Bilinear taps along the walk (2..64, or 1 at a sub-frame reach) — the
    /// same count the CPU oracle loops.
    pub steps: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DatamoshParams {
    intensity: f32,
    displacement: f32,
    bloom: f32,
    steps: i32,
}

impl FxEngine {
    /// Apply one echo/trails (docs/08 §3.13) to a linear working texture,
    /// returning a new texture of the same size. Starts the accumulator as
    /// the current frame (an `echo_accumulate` with weight 0 copies it), folds
    /// in each live tap's neighbour (looked up by offset `-(i+1)`), then mixes
    /// the trail back toward the current frame. A missing neighbour or a zero
    /// weight is skipped, so the pass cost tracks the live tap count.
    #[allow(clippy::too_many_arguments)]
    pub fn echo(
        &self,
        ctx: &GpuContext,
        current: &wgpu::Texture,
        neighbours: &[(i32, &wgpu::Texture)],
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &EchoOp,
    ) -> wgpu::Texture {
        // The Matte scales Decay per pixel: it rides on the tap
        // dispatches alone. The accumulator's first copy and the final Mix are
        // not the decay, so they take none.
        let params = |weight: f32, mode: u32, tap: i32, matted: bool| EchoParams {
            weight,
            mode,
            matte_on: f32::from(matted),
            tap,
        };
        // acc := current (weight 0 add = a + n*0 = a).
        let mut acc = work_texture(ctx, w, h, "fx-echo-acc");
        self.dispatch(
            ctx,
            &self.echo_accumulate,
            current,
            current,
            &acc,
            w,
            h,
            bytemuck::bytes_of(&params(0.0, 0, 0, false)),
        );
        for (i, &weight) in op.weights.iter().enumerate() {
            if weight <= 0.0 {
                continue;
            }
            let offset = -(i as i32 + 1);
            let Some((_, tex)) = neighbours.iter().find(|(o, _)| *o == offset) else {
                continue;
            };
            let next = work_texture(ctx, w, h, "fx-echo-acc");
            self.dispatch_matted(
                ctx,
                &self.echo_accumulate,
                &acc,
                tex,
                matte,
                &next,
                w,
                h,
                bytemuck::bytes_of(&params(weight, op.mode, i as i32, matte.is_some())),
            );
            acc = next;
        }
        let out = work_texture(ctx, w, h, "fx-echo-out");
        self.dispatch(
            ctx,
            &self.echo_mix,
            &acc,
            current,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params(op.mix, 0, 0, false)),
        );
        out
    }

    /// Apply one flow motion blur (docs/08 §3.2) to a linear working texture,
    /// returning a new texture of the same size. The Guertin-class
    /// reconstruction (docs/impl/optical-flow.md §4.5 item 3).
    ///
    /// **Two passes.** The first reduces `flow` to one dominant vector per
    /// `MB_TILE`-square tile; the second does the blur, and each pixel reads the
    /// 3×3 tile neighbourhood to learn what its surroundings are doing. That
    /// second direction is what lets a fast object smear *over* the still
    /// background it passes (v1, gathering only along each pixel's own vector,
    /// could not), and it is what an unconfident pixel borrows its direction
    /// from instead of freezing.
    ///
    /// The reduction lives here rather than at the upload seam deliberately:
    /// computing it from the flow texture the kernel already has keeps the whole
    /// change inside this crate — the decode worker, the render plan and the aux
    /// slots all still carry exactly one field.
    ///
    /// `flow`'s vectors are consumed exactly as
    /// `lumit_core::fx::cpu::motion_blur` reads its `u`/`v` slices, and the
    /// reduction matches `lumit_core::fx::cpu::motion_blur_tiles`, so the two
    /// agree (§1.6).
    /// Accumulation motion blur's average with a **per-pixel shutter** (docs/08
    /// §3.26): the same N sub-frame renders the equal-weight combine
    /// takes, but each pixel's weights decided by how far open the matte says
    /// its shutter is.
    ///
    /// **In plain terms.** Equal weights average all N moments, which is a fully
    /// open shutter. Where the matte is darker the average is taken over a
    /// shorter slice of those moments, shrunk toward `anchor` — where the
    /// frame's own time falls across the open shutter — so black is the
    /// unblurred frame and grey is a genuinely shorter exposure, which is not
    /// the same picture as a blurred frame faded back.
    ///
    /// `matte` must already be prepared (Channel picked, Invert applied): this
    /// effect draws no pass of its own, so it has no dispatch seam to do that
    /// at, and the caller does it once. `samples` must not be empty — a caller
    /// with nothing to average has nothing to call this for.
    pub fn accumulate_with_shutter(
        &self,
        ctx: &GpuContext,
        samples: &[wgpu::Texture],
        matte: &wgpu::Texture,
        w: u32,
        h: u32,
        anchor: f32,
    ) -> wgpu::Texture {
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct AccumShutterParams {
            anchor: f32,
            n: f32,
            k: f32,
            first: f32,
        }
        let n = samples.len().max(1) as f32;
        let mut acc = work_texture(ctx, w, h, "fx-accum-shutter");
        for (k, frame) in samples.iter().enumerate() {
            let next = work_texture(ctx, w, h, "fx-accum-shutter");
            self.dispatch_matted(
                ctx,
                &self.accum_shutter,
                &acc,
                frame,
                Some(matte),
                &next,
                w,
                h,
                bytemuck::bytes_of(&AccumShutterParams {
                    anchor,
                    n,
                    k: k as f32,
                    first: f32::from(k == 0),
                }),
            );
            acc = next;
        }
        acc
    }

    /// A supplied **Motion vectors** layer read as a dense flow field
    /// (docs/08 §3.2) — the GPU twin of
    /// `lumit_core::fx::cpu::motion_vectors_field`.
    ///
    /// **In plain terms.** A game engine or a 3D renderer already knows how
    /// every pixel moved and can hand that over as a picture: red is sideways,
    /// green is up-and-down, mid-grey is standing still. This turns that
    /// picture into the same field the flow engine measures, so
    /// [`Self::motion_blur`] and its tile reduction read one kind of field and
    /// know nothing about where it came from.
    ///
    /// `scale` is how many raster pixels a full channel means. Confidence comes
    /// out at 1 everywhere: a supplied vector is not a measurement that can
    /// have failed to match.
    pub fn motion_vectors_field(
        &self,
        ctx: &GpuContext,
        vectors: &wgpu::Texture,
        w: u32,
        h: u32,
        scale: f32,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let field = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx-mb-vectors-field"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // f32, as the measured field is: these vectors are compared
            // bit-for-bit against an f32 oracle.
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-mb-vectors-params"),
                contents: bytemuck::bytes_of(&MbTileParams {
                    tile: MB_TILE as i32,
                    vector_scale: scale,
                    _pad: [0; 2],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-mb-vectors-bind"),
            layout: &self.mb_tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(vectors)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(&field)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-mb-vectors-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-mb-vectors-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.mb_vectors);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        field
    }

    #[allow(clippy::too_many_arguments)]
    pub fn motion_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        flow: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &MotionBlurOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let tile = MB_TILE;
        let (tw, th) = (w.div_ceil(tile).max(1), h.div_ceil(tile).max(1));
        let tiles = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx-mb-tiles"),
            size: wgpu::Extent3d {
                width: tw,
                height: th,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // f32, not the working fp16: these vectors are compared bit-for-bit
            // against an f32 oracle.
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let out = work_texture(ctx, w, h, "fx-mb-out");
        let tbuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-mb-tile-params"),
                contents: bytemuck::bytes_of(&MbTileParams {
                    tile: tile as i32,
                    vector_scale: op.vector_scale,
                    _pad: [0; 2],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-mb-params"),
                contents: bytemuck::bytes_of(&MotionBlurParams {
                    shutter_frac: op.shutter_frac,
                    samples: op.samples.max(1),
                    mix_amt: op.mix,
                    view: op.view,
                    tile: tile as i32,
                    quality: op.quality,
                    matte_on: f32::from(matte.is_some()),
                    _pad: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let tile_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-mb-tile-bind"),
            layout: &self.mb_tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(flow)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(&tiles)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tbuf.as_entire_binding(),
                },
            ],
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-mb-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(&tiles)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(flow)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
                // The Matte. `None` binds `src` in its place, the same
                // "bound but not read" convention `dispatch_matted` uses.
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&view(matte.unwrap_or(src))),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-mb-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-mb-tile-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.mb_tilemax);
            cpass.set_bind_group(0, &tile_bind, &[]);
            cpass.dispatch_workgroups(tw.div_ceil(8), th.div_ceil(8), 1);
        }
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-mb-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.motion_blur);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }

    /// Apply Datamosh (docs/08 §3.12; its own effect, reworked to a
    /// flow-driven melt by T19) to a linear working texture, returning a new
    /// texture of the same size. One pass: per output pixel, a streamline walk
    /// of `op.steps` taps follows the `flow` field out of `prev` (re-sampling
    /// the flow each step, advancing ~one frame of motion), accumulating the
    /// samples with a `op.bloom` geometric weight, then blends the weighted
    /// mean over `cur` by Intensity. Shares [`Self::mb_layout`]/its pipeline
    /// layout with Motion blur (same three-sampled-input shape); its own
    /// pipeline and shader.
    #[allow(clippy::too_many_arguments)]
    pub fn datamosh(
        &self,
        ctx: &GpuContext,
        cur: &wgpu::Texture,
        prev: &wgpu::Texture,
        flow: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &DatamoshOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-dm-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-dm-params"),
                contents: bytemuck::bytes_of(&DatamoshParams {
                    intensity: op.intensity,
                    displacement: op.displacement,
                    bloom: op.bloom,
                    steps: op.steps,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-dm-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(cur)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(prev)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(flow)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
                // The Matte slot Motion blur added to this shared layout.
                // This kernel never reads it, so `cur` stands in it:
                // a binding cannot be left empty, and it is the same
                // "bound but not read" convention `dispatch_matted` uses.
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&view(cur)),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-dm-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-dm-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.datamosh);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
        out
    }
}
