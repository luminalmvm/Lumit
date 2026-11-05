//! The draws of a run of 3D layers: cut where they meet, and their depth.

use super::{
    scaled_size, Blend, CompositeLayer, Compositor, Cut, LayerUniform, Region, CUT_DEPTH,
    PIECE_CORNERS,
};
use crate::mesh::ModelPicture;
use crate::GpuContext;
use glam::Mat4;

impl Compositor {
    /// **Scene depth**: how far from the camera each pixel of the 3D layers is,
    /// metres, so an effect can read it. Each run of 3D layers keeps its
    /// nearest layer at each pixel, so layers that pass through each other cut
    /// as they do in the picture, and a run higher in the stack replaces what
    /// is below it wherever it covers, as it does in the picture. A pixel no
    /// 3D layer covers at least half reads as far as a half float goes, or as
    /// `under` where that is given: the depth of the layers below these.
    /// `metres_per_w` is the camera's zoom over its pixels per metre.
    /// Distance is counted from a clip w of `near`, so a scene far from the
    /// eye keeps its precision.
    ///
    /// 2D layers draw nothing here, so a 3D layer still reads through a 2D
    /// one in front of it. A 2D layer inside a collapsed 3D precomp lies on the
    /// precomp's plane, so it has a distance like any 3D layer.
    ///
    /// ponytail: track mattes and blend modes are not read here, so a layer
    /// a matte cuts away still has a distance. Read them if a scene needs it.
    #[allow(clippy::too_many_arguments)]
    pub fn composite_depth(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        runs: &[&[CompositeLayer<'_>]],
        camera: Mat4,
        metres_per_w: f32,
        near: f32,
        render_scale: f32,
        under: Option<&wgpu::Texture>,
    ) -> wgpu::Texture {
        self.composite_depth_joined(
            ctx,
            width,
            height,
            runs,
            camera,
            metres_per_w,
            near,
            render_scale,
            under,
            &[],
        )
    }

    /// As [`Self::composite_depth`], with `models`: a layer whose picture is
    /// one of theirs is a model's, placed full frame, and lays down the
    /// model's own depth at each pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn composite_depth_joined(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        runs: &[&[CompositeLayer<'_>]],
        camera: Mat4,
        metres_per_w: f32,
        near: f32,
        render_scale: f32,
        under: Option<&wgpu::Texture>,
        models: &[ModelPicture],
    ) -> wgpu::Texture {
        let (tw, th) = scaled_size(width, height, render_scale);
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("comp-depth"),
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
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let under = under.filter(|u| (u.width(), u.height()) == (tw, th));
        let pipelines = self.pipelines(ctx, 1, None);
        let joins = (!models.is_empty()).then(|| self.join_set(ctx, 1));
        let bind = |layer: &CompositeLayer<'_>, clear: bool| {
            if let Some(model) = models.iter().find(|m| &m.colour == layer.texture) {
                let params = [
                    (layer.opacity / 100.0).clamp(0.0, 1.0),
                    0.0,
                    near,
                    metres_per_w,
                ];
                let holdout = f32::from(u8::from(model.holdout));
                let flags = [f32::from(u8::from(clear)), holdout, 0.0, 0.0];
                let matrix = self.full_frame_matrix(width, height, None);
                return (
                    true,
                    self.join_group(ctx, model, matrix, params, flags, None),
                );
            }
            let uniform = LayerUniform {
                matrix: layer
                    .matrix(width as f32, height as f32, Some(&camera), None)
                    .to_cols_array_2d(),
                params: [
                    (layer.opacity / 100.0).clamp(0.0, 1.0),
                    f32::from(u8::from(clear)),
                    0.0,
                    0.0,
                ],
                target: [tw as f32, th as f32, near, metres_per_w],
                cut: [0.0; 4],
                piece: [[0.0; 4]; PIECE_CORNERS / 2],
            };
            let buffer = wgpu::util::DeviceExt::create_buffer_init(
                &ctx.device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("layer-depth-uniform"),
                    contents: bytemuck::bytes_of(&uniform),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            );
            let view = |t: &wgpu::Texture| t.create_view(&Default::default());
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("composite-depth-layer"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view(layer.texture)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&view(&self.white)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&view(&self.black)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&view(
                            layer.layer_mask.unwrap_or(&self.white),
                        )),
                    },
                ],
            });
            (false, group)
        };
        // Each run's 3D layers, after a clear of where they cover when the run
        // lies over something. The first run over nothing needs no clear.
        let mut over = under.is_some();
        type Draws = Vec<(bool, wgpu::BindGroup)>;
        let mut draws: Vec<(Draws, Draws)> = Vec::new();
        for run in runs {
            let solid: Vec<&CompositeLayer<'_>> = run.iter().filter(|l| l.three_d).collect();
            if solid.is_empty() {
                continue;
            }
            let clears = if over {
                solid.iter().map(|l| bind(l, true)).collect()
            } else {
                Vec::new()
            };
            draws.push((clears, solid.iter().map(|l| bind(l, false)).collect()));
            over = true;
        }
        let target_view = target.create_view(&Default::default());
        let mut encoder = ctx.encoder("composite-depth");
        if let Some(u) = under {
            encoder.copy_texture_to_texture(
                u.as_image_copy(),
                target.as_image_copy(),
                wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
            );
        }
        {
            let far = f64::from(half::f16::MAX.to_f32());
            let load = match under {
                Some(_) => wgpu::LoadOp::Load,
                None => wgpu::LoadOp::Clear(wgpu::Color {
                    r: far,
                    g: far,
                    b: far,
                    a: 1.0,
                }),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite-depth"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let (Some(depth), Some(clear)) = (&pipelines.depth, &pipelines.depth_clear) {
                for (clears, binds) in &draws {
                    for (join, (pipeline, list)) in
                        [4, 3].into_iter().zip([(clear, clears), (depth, binds)])
                    {
                        if list.is_empty() {
                            continue;
                        }
                        for (model, bind) in list {
                            // A model's own draw, the depth or the clear.
                            match joins.as_ref().filter(|_| *model) {
                                Some(j) => {
                                    pass.set_pipeline(&j[join]);
                                    pass.set_bind_group(0, bind, &[]);
                                }
                                None => {
                                    pass.set_pipeline(pipeline);
                                    pass.set_bind_group(0, bind, &[0]);
                                }
                            }
                            pass.draw(0..6, 0..1);
                        }
                    }
                }
            }
        }
        drop(encoder);
        target
    }

    /// As [`Self::composite_seeded`], with `cuts` (one per layer, or empty)
    /// from [`super::order::depth_order`]. A piece draws only that part of
    /// its layer. An unbroken run of layers cut by depth cuts where its
    /// layers meet, and a layer not cut that way ends the run.
    #[allow(clippy::too_many_arguments)]
    pub fn composite_cut(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompositeLayer<'_>],
        camera: Option<Mat4>,
        seed: Option<&wgpu::Texture>,
        render_scale: f32,
        samples: u32,
        region: Option<Region>,
        cuts: &[Option<Cut>],
    ) -> wgpu::Texture {
        self.composite_joined(
            ctx,
            width,
            height,
            background,
            layers,
            camera,
            seed,
            render_scale,
            samples,
            region,
            cuts,
            &[],
        )
    }

    /// As [`Self::composite_cut`], with `models`: a layer whose picture is one
    /// of theirs is a model's, placed full frame. In a run cut by depth each
    /// of its pixels sits at the model's own depth there, so models and 3D
    /// layers cut through each other.
    ///
    /// ponytail: a model draws Normal in a run cut by depth, whatever its
    /// blend mode, and takes no track matte or mask there.
    #[allow(clippy::too_many_arguments)]
    pub fn composite_joined(
        &self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompositeLayer<'_>],
        camera: Option<Mat4>,
        seed: Option<&wgpu::Texture>,
        render_scale: f32,
        samples: u32,
        region: Option<Region>,
        cuts: &[Option<Cut>],
        models: &[ModelPicture],
    ) -> wgpu::Texture {
        let model_of = |l: &CompositeLayer<'_>| models.iter().find(|m| &m.colour == l.texture);
        let cut_of = |i: usize| cuts.get(i).copied().flatten();
        let (tw, th) = match region {
            Some(r) => r.target_size(render_scale),
            None => scaled_size(width, height, render_scale),
        };
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("comp-frame"),
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
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        // The multisample colour attachment, when anti-aliasing is on: one
        // texture for the whole composite, whose contents persist between the
        // pass segments below (docs/impl/anti-aliasing.md §3). It is never read
        // by anything — every reader takes the resolved `target` — so it wants
        // no TEXTURE_BINDING, and `RENDER_ATTACHMENT` alone lets a driver keep
        // it in tiled memory on the hardware that can.
        // A run cut by depth needs a depth texture at the same count.
        let samples = if cuts.iter().any(|c| matches!(c, Some(Cut::Depth(_)))) {
            ctx.cut_sample_count(samples)
        } else {
            samples
        }
        .max(1);
        let msaa = (samples > 1).then(|| {
            self.attachment(
                ctx,
                &wgpu::TextureDescriptor {
                    label: Some("comp-frame-msaa"),
                    size: wgpu::Extent3d {
                        width: tw,
                        height: th,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: ctx.working(),
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
            )
        });
        let msaa_view = msaa.as_ref().map(|t| t.create_view(&Default::default()));
        let pipelines = self.pipelines(ctx, samples, None);
        // Where a pass draws, and where its result lands. Without MSAA the two
        // are the same texture and there is no resolve; with it, geometry goes
        // to the multisample attachment and each pass resolves into `target` —
        // which is what keeps the snapshot copy and the caller reading a
        // single-sample picture (trap 3).
        let attachment = |load: wgpu::LoadOp<wgpu::Color>| match &msaa_view {
            Some(ms) => wgpu::RenderPassColorAttachment {
                view: ms,
                resolve_target: Some(&view),
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            },
            None => wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            },
        };
        // Snapshot of the accumulation for shader-computed blends: one
        // per-frame scratch, copied into just before each such layer draws.
        // A holdout's plate is another, copied as its run starts.
        let needs_snapshot = layers.iter().any(|l| l.blend.uses_snapshot());
        let holdouts = models.iter().any(|m| m.holdout);
        let [snapshot, plate] = [needs_snapshot, holdouts].map(|needed| {
            needed.then(|| {
                ctx.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("comp-dst-snapshot"),
                    size: wgpu::Extent3d {
                        width: tw,
                        height: th,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: ctx.working(),
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            })
        });

        // Every draw's uniform in one buffer, each at its own offset.
        let size = std::mem::size_of::<LayerUniform>() as u64;
        let stride = wgpu::util::align_to(
            size,
            u64::from(ctx.device.limits().min_uniform_buffer_offset_alignment),
        );
        let mut bytes = vec![0u8; (stride * layers.len().max(1) as u64) as usize];
        for (i, layer) in layers.iter().enumerate() {
            let uniform = LayerUniform {
                matrix: match model_of(layer) {
                    Some(_) => self.full_frame_matrix(width, height, region),
                    None => layer
                        .matrix(width as f32, height as f32, camera.as_ref(), region)
                        .to_cols_array_2d(),
                },
                params: [
                    (layer.opacity / 100.0).clamp(0.0, 1.0),
                    f32::from(layer.matte.is_some()),
                    f32::from(layer.matte.as_ref().is_some_and(|m| m.luma)),
                    f32::from(layer.matte.as_ref().is_some_and(|m| m.inverted)),
                ],
                target: [
                    tw as f32,
                    th as f32,
                    layer.blend.snapshot_mode(),
                    f32::from(layer.premultiplied),
                ],
                cut: match cut_of(i) {
                    Some(Cut::Depth(z)) => [z, 0.0, 0.0, 0.0],
                    Some(Cut::Piece { count, .. }) => [0.0, count as f32, 0.0, 0.0],
                    None => [0.0; 4],
                },
                piece: match cut_of(i) {
                    Some(Cut::Piece { corners, .. }) => std::array::from_fn(|k| {
                        let (a, b) = (corners[2 * k], corners[2 * k + 1]);
                        [a[0], a[1], b[0], b[1]]
                    }),
                    _ => [[0.0; 4]; PIECE_CORNERS / 2],
                },
            };
            bytes[i * stride as usize..][..size as usize]
                .copy_from_slice(bytemuck::bytes_of(&uniform));
        }
        let uniforms = wgpu::util::DeviceExt::create_buffer_init(
            &ctx.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("layer-uniforms"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        // The draws of one picture, the pieces of a split layer, share one
        // bind group.
        type Picture = (
            wgpu::Texture,
            Option<wgpu::Texture>,
            Option<wgpu::Texture>,
            bool,
        );
        // A texture hashes by its id, which never changes.
        #[allow(clippy::mutable_key_type)]
        let mut pictures: std::collections::HashMap<Picture, usize> =
            std::collections::HashMap::new();
        let mut groups: Vec<wgpu::BindGroup> = Vec::new();
        let picture_of: Vec<usize> = layers
            .iter()
            .map(|layer| {
                let key = (
                    layer.texture.clone(),
                    layer.matte.as_ref().map(|m| m.texture.clone()),
                    layer.layer_mask.cloned(),
                    layer.blend.uses_snapshot(),
                );
                *pictures.entry(key).or_insert_with(|| {
                    groups.push(
                        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("composite-layer"),
                            layout: &self.layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(
                                        &layer.texture.create_view(&Default::default()),
                                    ),
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
                                        size: wgpu::BufferSize::new(size),
                                    }),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(
                                        &layer
                                            .matte
                                            .as_ref()
                                            .map(|m| m.texture)
                                            .unwrap_or(&self.white)
                                            .create_view(&Default::default()),
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: wgpu::BindingResource::TextureView(
                                        &layer
                                            .layer_mask
                                            .unwrap_or(&self.white)
                                            .create_view(&Default::default()),
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::TextureView(
                                        &if layer.blend.uses_snapshot() {
                                            snapshot.as_ref().unwrap_or(&self.black)
                                        } else {
                                            &self.black
                                        }
                                        .create_view(&Default::default()),
                                    ),
                                },
                            ],
                        }),
                    );
                    groups.len() - 1
                })
            })
            .collect();

        let mut encoder = ctx.encoder("composite");

        // Draw in pass segments: shader-computed blends need the accumulated
        // target copied out first (a copy cannot happen inside a pass).
        let clear = wgpu::LoadOp::Clear(wgpu::Color {
            r: background[0],
            g: background[1],
            b: background[2],
            a: background[3],
        });
        // A seed replaces the clear: the target starts as the previous
        // stage's pixels, and every pass loads instead of clearing.
        //
        // Without MSAA that is a straight texture copy, as it always was. With
        // it the copy is invalid — `copy_texture_to_texture` cannot cross sample
        // counts (trap 1) — so the seed is *drawn* full-frame into the
        // multisample attachment instead, by `fs_seed`, which reads its own
        // pixel with `textureLoad` and so reproduces the seed exactly.
        // `seed_keep` holds the uniform buffer and bind group alive until submit.
        let mut seed_keep: Option<(wgpu::Buffer, wgpu::BindGroup)> = None;
        if let Some(s) = seed {
            match &msaa_view {
                None => encoder.copy_texture_to_texture(
                    s.as_image_copy(),
                    target.as_image_copy(),
                    wgpu::Extent3d {
                        width: tw,
                        height: th,
                        depth_or_array_layers: 1,
                    },
                ),
                Some(ms) => {
                    let uniform = LayerUniform {
                        matrix: self.full_frame_matrix(width, height, None),
                        params: [1.0, 0.0, 0.0, 0.0],
                        target: [tw as f32, th as f32, -1.0, 0.0],
                        cut: [0.0; 4],
                        piece: [[0.0; 4]; PIECE_CORNERS / 2],
                    };
                    let buffer = wgpu::util::DeviceExt::create_buffer_init(
                        &ctx.device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("composite-seed-uniform"),
                            contents: bytemuck::bytes_of(&uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        },
                    );
                    let white_view = self.white.create_view(&Default::default());
                    let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("composite-seed"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(
                                    &s.create_view(&Default::default()),
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: buffer.as_entire_binding(),
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
                                resource: wgpu::BindingResource::TextureView(
                                    &self.black.create_view(&Default::default()),
                                ),
                            },
                        ],
                    });
                    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("composite-seed"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: ms,
                            resolve_target: Some(&view),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    if let Some(seed) = &pipelines.seed {
                        rpass.set_pipeline(seed);
                    }
                    rpass.set_bind_group(0, &bind, &[0]);
                    rpass.draw(0..6, 0..1);
                    drop(rpass);
                    seed_keep = Some((buffer, bind));
                }
            }
        }
        // A run cut by depth draws in three goes. Each layer lays its depth
        // down, so the nearest one at every pixel is known. Then all that
        // lies behind that nearest one draws, farthest first, and then the
        // nearest one over it. What is solid comes out right whatever the
        // order, and where see-through layers meet, all but the front one go
        // by the sort.
        #[derive(Clone, Copy, PartialEq)]
        enum Go {
            Plain,
            Depth,
            Behind,
            Front,
        }
        let by_depth = |j: usize| matches!(cut_of(j), Some(Cut::Depth(_)));
        let mut draws: Vec<(usize, Go, Option<usize>)> = Vec::with_capacity(layers.len());
        let mut i = 0usize;
        while i < layers.len() {
            let end = (i..layers.len())
                .find(|&j| !by_depth(j))
                .unwrap_or(layers.len());
            if end == i {
                draws.push((i, Go::Plain, None));
                i += 1;
                continue;
            }
            for go in [Go::Depth, Go::Behind, Go::Front] {
                draws.extend((i..end).map(|j| (j, go, Some(i))));
            }
            i = end;
        }
        // The depth they test against, made only when a layer is cut.
        let cut_depth = (draws.len() > layers.len()).then(|| {
            let depth = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("comp-cut-depth"),
                size: wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: CUT_DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            ctx.charge_vram(
                crate::texture_bytes(CUT_DEPTH, tw, th).saturating_mul(u64::from(samples)),
            );
            (
                self.pipelines(ctx, samples, Some(wgpu::CompareFunction::Less)),
                self.pipelines(ctx, samples, Some(wgpu::CompareFunction::Equal)),
                depth.create_view(&Default::default()),
            )
        });
        // A model in a run cut by depth draws its own way, each pixel at the
        // model's own depth.
        let joins =
            (cut_depth.is_some() && !models.is_empty()).then(|| self.join_set(ctx, samples));
        let joined: Vec<Option<wgpu::BindGroup>> = match joins {
            Some(_) => layers
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let (Some(Cut::Depth(z)), Some(model)) = (cut_of(i), model_of(l)) else {
                        return None;
                    };
                    let params = [(l.opacity / 100.0).clamp(0.0, 1.0), z, 0.0, 0.0];
                    let matrix = self.full_frame_matrix(width, height, region);
                    let flags = [0.0, f32::from(u8::from(model.holdout)), 0.0, 0.0];
                    let under = plate.as_ref().filter(|_| model.holdout);
                    Some(self.join_group(ctx, model, matrix, params, flags, under))
                })
                .collect(),
            None => Vec::new(),
        };
        // A draw that reads the snapshot, which has to be taken first, unless
        // it is the next piece of the picture just drawn: pieces of a layer
        // never overlap, so they read the same one.
        let piece = |idx: usize| matches!(cut_of(idx), Some(Cut::Piece { .. }));
        let snaps = |k: usize| {
            let (idx, go, _) = draws[k];
            go != Go::Depth
                && layers[idx].blend.uses_snapshot()
                && !(k > 0 && piece(idx) && piece(draws[k - 1].0) && {
                    let before = draws[k - 1].0;
                    picture_of[before] == picture_of[idx] && before != idx
                })
        };
        // The first draw of a run with a holdout, where what is under the run
        // is kept for it.
        let plate_at = |k: usize| {
            let run = draws[k].2;
            run.is_some()
                && (k == 0 || draws[k - 1].2 != run)
                && draws.iter().any(|&(j, _, r)| {
                    r == run
                        && models
                            .iter()
                            .any(|m| m.holdout && &m.colour == layers[j].texture)
                })
        };
        let mut cleared: Option<usize> = None;
        let mut first_pass = seed.is_none();
        let mut k = 0usize;
        while k < draws.len() {
            let run = draws[k].2;
            for (wanted, into) in [(snaps(k), &snapshot), (plate_at(k), &plate)] {
                if let (true, Some(snap)) = (wanted, into) {
                    if first_pass {
                        // Materialise the background before snapshotting. With
                        // MSAA this pass resolves too, so the snapshot copy
                        // below reads a real single-sample background.
                        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("composite-clear"),
                            color_attachments: &[Some(attachment(clear))],
                            ..Default::default()
                        });
                        first_pass = false;
                    }
                    encoder.copy_texture_to_texture(
                        target.as_image_copy(),
                        snap.as_image_copy(),
                        wgpu::Extent3d {
                            width: tw,
                            height: th,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
            // One pass: this draw plus any following fixed-function draws,
            // and only those of the same run, if it has one.
            let mut end = k + 1;
            while end < draws.len() && !snaps(end) && draws[end].2 == run {
                end += 1;
            }
            let depth = match (run, &cut_depth) {
                (Some(run), Some((_, _, view))) => {
                    // A run starts with nothing in the way.
                    let load = if cleared == Some(run) {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(0.0)
                    };
                    cleared = Some(run);
                    Some(wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    })
                }
                _ => None,
            };
            {
                // `LoadOp::Load` loads the MULTISAMPLE attachment, whose
                // contents persist between passes — not the resolve target,
                // which the API offers no way to load from and which is not
                // what is wanted anyway (docs/impl/anti-aliasing.md §3).
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("composite"),
                    color_attachments: &[Some(attachment(if first_pass {
                        clear
                    } else {
                        wgpu::LoadOp::Load
                    }))],
                    depth_stencil_attachment: depth,
                    ..Default::default()
                });
                first_pass = false;
                for &(idx, go, _) in &draws[k..end] {
                    if let (Some(Some(group)), Some(j)) = (joined.get(idx), &joins) {
                        let holdout = model_of(&layers[idx]).is_some_and(|m| m.holdout);
                        rpass.set_pipeline(
                            &j[match (go, holdout) {
                                (Go::Depth, _) => 0,
                                (Go::Front, false) => 1,
                                (Go::Front, true) => 5,
                                (_, false) => 2,
                                (_, true) => 6,
                            }],
                        );
                        rpass.set_bind_group(0, group, &[]);
                        rpass.draw(0..6, 0..1);
                        continue;
                    }
                    let set = match (go, &cut_depth) {
                        (Go::Behind, Some((behind, _, _))) => behind,
                        (Go::Depth | Go::Front, Some((_, front, _))) => front,
                        _ => &pipelines,
                    };
                    let pipeline = match (go, layers[idx].blend) {
                        (Go::Depth, _) => set.depth.as_ref(),
                        (_, Blend::Normal) => Some(&set.normal),
                        (_, Blend::Add) => Some(&set.add),
                        (_, Blend::Multiply) => Some(&set.multiply),
                        (_, Blend::Plus) => Some(&set.plus),
                        _ => Some(&set.snapshot),
                    };
                    let Some(pipeline) = pipeline else {
                        continue;
                    };
                    // A piece draws as a fan of triangles over its corners.
                    let vertices = match cut_of(idx) {
                        Some(Cut::Piece { count, .. }) => count.saturating_sub(2) as u32 * 3,
                        _ => 6,
                    };
                    rpass.set_pipeline(pipeline);
                    rpass.set_bind_group(
                        0,
                        &groups[picture_of[idx]],
                        &[(idx as u64 * stride) as u32],
                    );
                    rpass.draw(0..vertices, 0..1);
                }
            }
            k = end;
        }
        if first_pass {
            // No layers at all: still clear to the background.
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite-clear"),
                color_attachments: &[Some(attachment(clear))],
                ..Default::default()
            });
        }
        // Recording ends here; whether the buffer goes to the driver now or at
        // the end of the frame's batch is the encoder guard's business.
        // The seed's buffer and bind group are released after it either way —
        // the recorded commands hold their own references to what they use, so
        // the deferred submission still has them.
        drop(encoder);
        drop(seed_keep);
        if let Some(msaa) = msaa {
            self.give_back(ctx, msaa);
        }
        target
    }
}
