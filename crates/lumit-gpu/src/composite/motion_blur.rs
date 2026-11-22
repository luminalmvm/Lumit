//! Per-layer motion blur: one layer drawn at its sub-frame placements and
//! averaged (docs/06 §4).

use super::{
    scaled_size, Blend, CompositeLayer, Compositor, LayerUniform, MbSample, PIECE_CORNERS,
};
use crate::GpuContext;
use glam::Mat4;

impl Compositor {
    /// The premultiplied average of one layer drawn at N sub-frame placements —
    /// the per-layer motion-blur smear (docs/06 §4).
    ///
    /// The SAME `texture` is composited at each [`MbSample`] placement into a
    /// fresh transparent comp-sized target with a pure-additive blend (BOTH
    /// colour and alpha add) at weight `1/N`, so the target holds
    /// `(1/N)·Σ premul(sample_k)` — the arithmetic mean of the premultiplied
    /// sub-frame images. A static layer (every placement equal) averages back
    /// to itself, alpha and all; a moving one smears, its coverage translucent
    /// in proportion to how much of the shutter each pixel was covered.
    ///
    /// `three_d`, `pre` and `camera` place every sub-copy exactly as the
    /// layer's own draw would be placed, a sample's own `pre` standing in for
    /// the layer's when its parents or the camera move. The caller composites the returned comp-sized texture 1:1
    /// (identity placement, `size = (width, height)`) carrying the layer's real
    /// blend, opacity, matte and mask, so those apply once to the averaged
    /// image, never per sub-copy.
    ///
    /// This is the single helper both the preview and the export path call, so
    /// per-layer motion blur is identical between them. An empty
    /// `samples` returns a transparent frame (the caller only invokes this with
    /// a non-empty set, so that is a defensive no-op, never a panic).
    ///
    /// `render_scale` splits logical from actual exactly as
    /// [`Self::composite_seeded`] does: placements stay in LOGICAL comp
    /// pixels, the smear target and fp32 accumulators take the ACTUAL
    /// [`scaled_size`] dims. Export passes 1.0.
    ///
    /// `aa_samples` is the project's anti-aliasing count, and it must be the
    /// same one the composite is drawn at: each sub-frame placement is a quad
    /// placed exactly as the layer's own draw would be, so an anti-aliased
    /// composite with an aliased smear would show the seam on every blurring
    /// layer (docs/impl/anti-aliasing.md, trap 4). The fp32 add and resolve
    /// passes are full-frame with no geometry of their own and stay
    /// single-sample, reading the resolved placement.
    #[allow(clippy::too_many_arguments)]
    pub fn motion_blur_average(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        texture: &wgpu::Texture,
        premultiplied: bool,
        size: (f32, f32),
        samples: &[MbSample],
        three_d: bool,
        pre: Option<[[f32; 4]; 4]>,
        camera: Option<Mat4>,
        render_scale: f32,
        aa_samples: u32,
    ) -> wgpu::Texture {
        let (tw, th) = scaled_size(width, height, render_scale);
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mb-average"),
            size: wgpu::Extent3d {
                width: tw,
                height: th,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ctx.working(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        // Equal weights sum to 1: N copies each contributing 1/N. The weight is
        // NOT baked into the placement (which renders at full alpha into the
        // fp16 temp) — it is applied in f32 by the add pass, so a static layer
        // stays bit-exact (docs/06 §4).
        let weight_frac = if samples.is_empty() {
            0.0
        } else {
            1.0 / samples.len() as f32
        };
        let src_view = texture.create_view(&Default::default());
        let white_view = self.white.create_view(&Default::default());
        let black_view = self.black.create_view(&Default::default());
        // Every copy's uniform in one buffer, each at its own offset, read
        // through one bind group. A bind group per copy held one of the
        // driver's few sampler slots each until the frame was done, and a
        // frame of fast layers ran out of them.
        let one = std::mem::size_of::<LayerUniform>() as u64;
        let stride = wgpu::util::align_to(
            one,
            u64::from(ctx.device.limits().min_uniform_buffer_offset_alignment),
        );
        let mut bytes = vec![0u8; (stride * samples.len().max(1) as u64) as usize];
        for (i, s) in samples.iter().enumerate() {
            let layer = CompositeLayer {
                texture,
                size,
                position: s.position,
                anchor: s.anchor,
                scale: s.scale,
                rotation_deg: s.rotation_deg,
                opacity: 100.0,
                matte: None,
                blend: Blend::Add,
                z: s.z,
                rotation_x_deg: s.rotation_x_deg,
                rotation_y_deg: s.rotation_y_deg,
                three_d,
                layer_mask: None,
                pre: s.pre.or(pre),
                premultiplied,
            };
            let uniform = LayerUniform {
                matrix: layer
                    // No region: a motion-blur sub-copy is averaged into a
                    // comp-sized texture and the caller composites THAT,
                    // where the region applies once.
                    .matrix(width as f32, height as f32, camera.as_ref(), None)
                    .to_cols_array_2d(),
                // No matte on a sub-copy: the layer's matte applies to the
                // averaged result at the caller's 1:1 composite. Full alpha —
                // the 1/N weight is applied later, in f32, by the add pass.
                params: [1.0, 0.0, 0.0, 0.0],
                target: [tw as f32, th as f32, -1.0, f32::from(premultiplied)],
                cut: [0.0; 4],
                piece: [[0.0; 4]; PIECE_CORNERS / 2],
            };
            bytes[i * stride as usize..][..one as usize]
                .copy_from_slice(bytemuck::bytes_of(&uniform));
        }
        let uniforms = wgpu::util::DeviceExt::create_buffer_init(
            &ctx.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("mb-sample-uniforms"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mb-sample"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(one),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&black_view),
                },
            ],
        });
        let offsets: Vec<u32> = (0..samples.len())
            .map(|i| (i as u64 * stride) as u32)
            .collect();

        // fp32 accumulation (docs/06 §4). Additive blend into an fp16 target
        // drifts a LSB on a static layer's edges; fp32 blend into a render
        // target is unavailable, and each placement is a transformed (partial)
        // quad, so the accumulate() full-frame ping-pong can't apply. Instead:
        // render each placement into a cleared fp16 temp (0 outside its quad),
        // add that temp into an fp32 running sum full-frame, and resolve once.
        let mk_f32 = |label: &str| {
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let temp = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mb-temp"),
            size: wgpu::Extent3d {
                width: tw,
                height: th,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ctx.working(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let temp_view = temp.create_view(&Default::default());
        // Trap 4: the placement quads are geometry, so they get the same
        // multisample treatment the composite does, resolving into `temp` —
        // which is what the fp32 add pass then reads, single-sample as ever.
        let aa_samples = aa_samples.max(1);
        // The composite's kept attachment when the shapes agree: every
        // placement clears it before drawing.
        let temp_msaa = (aa_samples > 1).then(|| {
            self.attachment(
                ctx,
                &wgpu::TextureDescriptor {
                    label: Some("mb-temp-msaa"),
                    size: wgpu::Extent3d {
                        width: tw,
                        height: th,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: aa_samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: ctx.working(),
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
            )
        });
        let temp_msaa_view = temp_msaa
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let pipelines = self.pipelines(ctx, aa_samples, None);
        let accum_a = mk_f32("mb-accum-a");
        let accum_b = mk_f32("mb-accum-b");
        let a_view = accum_a.create_view(&Default::default());
        let b_view = accum_b.create_view(&Default::default());

        // Identity full-frame placement for the add and resolve passes.
        let ident_matrix = self.full_frame_matrix(width, height, None);
        // Full-frame uniform for the add/resolve passes; `px` is the add weight
        // (1/N), applied in f32 by fs_add_f32 (fs_copy_f32 ignores it).
        let full_uniform = |px: f32| {
            let u = LayerUniform {
                matrix: ident_matrix,
                params: [px, 0.0, 0.0, 0.0],
                target: [tw as f32, th as f32, -1.0, 0.0],
                cut: [0.0; 4],
                piece: [[0.0; 4]; PIECE_CORNERS / 2],
            };
            wgpu::util::DeviceExt::create_buffer_init(
                &ctx.device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("mb-fullframe-uniform"),
                    contents: bytemuck::bytes_of(&u),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            )
        };

        // The add pass reads the temp and one side of the running sum, so two
        // bind groups serve every batch, one for each side.
        let ubuf = full_uniform(weight_frac);
        let add_bind = |prev_view: &wgpu::TextureView| {
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mb-add"),
                layout: &self.accum_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&temp_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(prev_view),
                    },
                ],
            })
        };
        let (add_from_a, add_from_b) = (add_bind(&a_view), add_bind(&b_view));
        let mut encoder = ctx.encoder("mb-average");
        // The running sum starts at zero.
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mb-accum-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &a_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        let mut prev_is_a = true;
        // A few placements are summed into the temp at a time, so a fast
        // layer drawn a hundred times does not pay a full-frame add for each.
        // Four keep the half-float sum within a tenth of a percent.
        for batch in offsets.chunks(4) {
            // Sum these placements into a cleared temp: premul(source) where
            // each quad falls, 0 elsewhere.
            {
                let (draw_view, resolve) = match &temp_msaa_view {
                    Some(ms) => (ms, Some(&temp_view)),
                    None => (&temp_view, None),
                };
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mb-placement"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: draw_view,
                        resolve_target: resolve,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                rp.set_pipeline(&pipelines.plus);
                for &offset in batch {
                    rp.set_bind_group(0, &bind, &[offset]);
                    rp.draw(0..6, 0..1);
                }
            }
            // Add the temp into the fp32 running sum (whole frame at once).
            let (add_bind, next_view) = if prev_is_a {
                (&add_from_a, &b_view)
            } else {
                (&add_from_b, &a_view)
            };
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mb-add"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: next_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                rp.set_pipeline(&self.pipeline_add_f32);
                rp.set_bind_group(0, add_bind, &[]);
                rp.draw(0..6, 0..1);
            }
            prev_is_a = !prev_is_a;
        }

        // Resolve the fp32 sum (in accum_a when the next write would target it)
        // back to the working format.
        let sum_view = if prev_is_a { &a_view } else { &b_view };
        let rbuf = full_uniform(1.0);
        let copy_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mb-resolve"),
            layout: &self.accum_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: rbuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(sum_view),
                },
            ],
        });
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mb-resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rp.set_pipeline(&self.pipeline_accum_copy);
            rp.set_bind_group(0, &copy_bind, &[]);
            rp.draw(0..6, 0..1);
        }
        drop(encoder);
        drop((uniforms, ubuf, rbuf));
        if let Some(msaa) = temp_msaa {
            self.give_back(ctx, msaa);
        }
        target
    }
}
