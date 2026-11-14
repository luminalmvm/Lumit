//! One layer's picture: its source made ready, then its effect stack run.

use super::{straight_to_composite, upload_roto_matte, Realiser};
use crate::draw::{CompLayerDraw, DrawSource};

impl Realiser<'_> {
    /// A Precomp layer read as another's input: its comp with the strokes on
    /// the layer itself over it, as the layer's own picture has them.
    pub(super) fn painted_input(&self, n: &crate::draw::NestedInputDraw) -> wgpu::Texture {
        let nested =
            self.realise_nested(n.key, n.camera, n.width, n.height, n.background, &n.draws);
        let (w, h) = (f64::from(n.width), f64::from(n.height));
        self.paint_over(nested, w, h, &n.paint, n.paint_time)
    }

    /// A Precomp on Flow: the picture `phase` of the way from `a` to `b`,
    /// measured on the card and painted by the synthesis footage on Flow is
    /// painted by, in linear float so nothing above white is lost. Without a
    /// flow engine, or if either step fails, the two are mixed. With RIFE the
    /// model paints it where it can, and where it cannot preview says so on
    /// `layer`'s row and an export fails the frame.
    ///
    /// ponytail: both pictures come back off the card and the painting runs on
    /// the processor, the float path footage takes too. A painting pass that
    /// reads textures would keep it on the card.
    fn flow_between(
        &self,
        layer: uuid::Uuid,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        phase: f32,
        params: &lumit_core::retime::FlowParams,
    ) -> wgpu::Texture {
        let (w, h) = (a.width(), a.height());
        if let (lumit_core::retime::FlowEngineChoice::Rife, Some(held)) = (params.engine, self.flow)
        {
            match self.model_between(held, a, b, phase) {
                Ok(painted) => {
                    crate::decode::note_synthesis(layer, None);
                    return painted;
                }
                Err(why) if held.borrow().refuse => {
                    held.borrow_mut().refused = Some(crate::decode::refusal_sentence(&why));
                }
                Err(why) => crate::decode::note_synthesis(layer, Some(why)),
            }
        }
        let painted = || {
            let set = crate::decode::flow_settings(params);
            let mut held = self.flow?.borrow_mut();
            let engine = held
                .engine
                .get_or_insert_with(|| lumit_flow::FlowEngine::with_context(&self.ctx));
            // The solver and the reads run on encoders of their own.
            self.ctx.flush();
            let (fwd, bwd) = engine.flow_pair_textures(a, b, &set)?;
            let (fwd, bwd) = (
                lumit_flow::borrow_uncertain(&fwd, &bwd),
                lumit_flow::borrow_uncertain(&bwd, &fwd),
            );
            let bytes = |t: &wgpu::Texture| {
                let px = self.engine.readback_linear_f32(&self.ctx, t).ok()?;
                Some(px.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
            };
            let (pa, pb) = (bytes(a)?, bytes(b)?);
            let px =
                engine.synthesize_at_f32(&pa, &pb, w as usize, h as usize, &fwd, &bwd, phase, &set);
            Some(self.engine.upload_float_frame(&self.ctx, &px, w, h))
        };
        painted().unwrap_or_else(|| {
            self.compositor
                .accumulate(&self.ctx, w, h, &[(a, 1.0 - phase), (b, phase)])
        })
    }

    /// The picture between `a` and `b` as the model paints it, or why it did
    /// not. The model reads eight-bit colour, so a picture with anything above
    /// white is refused as a float source is. It has no alpha either, so a
    /// picture with gaps in it has its coverage painted again as grey.
    fn model_between(
        &self,
        held: &std::cell::RefCell<super::CompositeFlow>,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        phase: f32,
    ) -> Result<wgpu::Texture, crate::decode::SynthesisRefusal> {
        use lumit_core::pixels::{srgb_decode, srgb_encode};
        let (w, h) = (a.width(), a.height());
        // Asked first, so a machine with no pack reads nothing back.
        crate::decode::opened(&mut held.borrow_mut().synthesis)?;
        self.ctx.flush();
        let read = |t: &wgpu::Texture| {
            let px = self.engine.readback_linear_f32(&self.ctx, t);
            px.map_err(|e| crate::decode::SynthesisRefusal::Failed(e.to_string()))
        };
        let (pa, pb) = (read(a)?, read(b)?);
        let both = || pa.chunks_exact(4).chain(pb.chunks_exact(4));
        if both().any(|p| p[..3].iter().any(|v| *v > 1.0)) {
            return Err(crate::decode::SynthesisRefusal::FloatSource);
        }
        let colour = |px: &[f32]| -> Vec<u8> {
            let rgb = |p: &[f32]| [srgb_encode(p[0]), srgb_encode(p[1]), srgb_encode(p[2]), 255];
            px.chunks_exact(4).flat_map(rgb).collect()
        };
        let cover = |px: &[f32]| -> Vec<u8> {
            let grey = |p: &[f32]| [(p[3].clamp(0.0, 1.0) * 255.0).round() as u8; 4];
            px.chunks_exact(4).flat_map(grey).collect()
        };
        let (pw, ph) = (w as usize, h as usize);
        let mut flow = held.borrow_mut();
        let door = &mut flow.synthesis;
        let paint = |x: &[u8], y: &[u8], door: &mut _| {
            crate::decode::synthesised(door, x, y, pw, ph, phase, false)
        };
        let rgb = paint(&colour(&pa), &colour(&pb), door)?;
        let opaque = both().all(|p| p[3] >= 1.0);
        // An Add blend can leave colour above alpha, which is kept.
        let bounded = both().all(|p| p[..3].iter().all(|v| *v <= p[3]));
        let alpha = if opaque {
            None
        } else {
            Some(paint(&cover(&pa), &cover(&pb), door)?)
        };
        let px: Vec<u8> = rgb
            .chunks_exact(4)
            .enumerate()
            .flat_map(|(i, p)| {
                let al = alpha
                    .as_ref()
                    .and_then(|m| m.get(i * 4))
                    .map_or(1.0, |v| f32::from(*v) / 255.0);
                let c = |v: u8| srgb_decode(v).min(if bounded { al } else { 1.0 });
                [c(p[0]), c(p[1]), c(p[2]), al]
            })
            .flat_map(f32::to_le_bytes)
            .collect();
        Ok(self.engine.upload_float_frame(&self.ctx, &px, w, h))
    }

    /// `l`'s picture before it is lit or placed, how much its effects grew
    /// it, and the depth the mesh pass drew beside it when it is a model,
    /// which takes `shadows` and the comp's environment. `fx_ms` gathers each effect's own time when the
    /// frame is profiled.
    pub(super) fn layer_picture(
        &self,
        l: &CompLayerDraw,
        camera: Option<lumit_core::model::CameraPose>,
        (width, height): (u32, u32),
        shadows: (
            Option<&lumit_gpu::fx::lighting::ShadowCasters>,
            Option<&lumit_gpu::mesh::shadow::ShadowMaps>,
            Option<&lumit_gpu::fx::environment::EnvironmentLight>,
        ),
        fx_ms: &mut Option<Vec<f32>>,
    ) -> (wgpu::Texture, (f32, f32), Option<wgpu::Texture>) {
        let mut model_depth = None;
        let tex = match &l.source {
            DrawSource::Pixels {
                rgba,
                tex_w,
                tex_h,
                format,
                colour_space,
            } if straight_to_composite(l, camera)
                && (shadows.2.is_none() || !l.three_d)
                && matches!(format, lumit_media::decode::PixelFormat::Srgb8)
                && self.input_transform(colour_space).is_none() =>
            {
                // Nothing between here and the composite draw, so the
                // eight-bit upload goes there as it arrived. The sampler
                // decodes sRGB in hardware, which is the decode the
                // linearise pass was asking it for anyway, so the picture
                // is the same one and the working-format copy is never
                // made.
                let upload = |rgba: &[u8], w, h| self.engine.upload_srgb8(&self.ctx, rgba, w, h);
                self.kept_source(rgba, *tex_w, *tex_h, *format, colour_space, true, upload)
            }
            DrawSource::Pixels {
                rgba,
                tex_w,
                tex_h,
                format,
                colour_space,
            } => {
                let upload =
                    |rgba: &[u8], w, h| self.upload_source(rgba, w, h, *format, colour_space);
                self.kept_source(rgba, *tex_w, *tex_h, *format, colour_space, false, upload)
            }
            DrawSource::Nested {
                width,
                height,
                background,
                draws,
                camera,
                key,
                paint,
                paint_time,
                flow,
            } => {
                let mut nested =
                    self.realise_nested(*key, *camera, *width, *height, *background, draws);
                // A Precomp on Flow: the frame after, and the picture between.
                if let Some(f) = flow {
                    let then = self.realise(f.camera, *width, *height, *background, &f.draws);
                    nested = self.flow_between(l.layer, &nested, &then, f.phase, &f.params);
                }
                self.paint_over(
                    nested,
                    f64::from(*width),
                    f64::from(*height),
                    paint,
                    *paint_time,
                )
            }
            // **A node graph composition** (docs/impl/node-graph-comp.md
            // §2.3). Its frame is allocated at the render scale like every
            // comp's, and the walk places its Read boxes in the graph's
            // own pixels, so the scale applies once and a graph renders at
            // half preview what a layer comp of the same picture does.
            DrawSource::Graph(plan) => {
                let (gw, gh) =
                    lumit_gpu::composite::scaled_size(plan.width, plan.height, self.render_scale);
                self.realise_graph(plan, None, gw, gh, self.render_scale)
            }
            DrawSource::Model(m) => {
                match self.model_picture(l, m, camera, (width, height), shadows) {
                    Some(p) => {
                        model_depth = Some(p.depth);
                        p.colour
                    }
                    None => self.transparent(1, 1),
                }
            }
            DrawSource::Adjust | DrawSource::Haze(_) | DrawSource::Environment(_) => {
                // realise splits segments at every Adjust draw, so none
                // reaches here; a transparent texel keeps the no-panic
                // rule (and draws nothing) if that ever regresses.
                let src = self.engine.upload_srgb8(&self.ctx, &[0, 0, 0, 0], 1, 1);
                self.engine.linearise(&self.ctx, &src)
            }
        };
        // The raster the stack starts from, kept because one effect can
        // grow it and the composite must then place a picture wider
        // than the layer it came from.
        let (source_w, source_h) = (tex.width(), tex.height());
        // The effect stack runs on the linear source, after masks and
        // before the transform (docs/08 §1.5; docs/06 render order).
        let tex = if l.fx.is_empty() {
            tex
        } else {
            let (w, h) = (tex.width(), tex.height());
            // Neighbour source frames a temporal effect (echo) reads;
            // empty for a plain stack, so this uploads nothing then.
            // **A Precomp measures its own motion** (docs/08 §3.2).
            // A comp has no decoded frames either, so a Motion blur or
            // Datamosh on a Precomp layer bound nothing and passed through;
            // the nested picture was built again at each neighbour time, and
            // the pair is compared here. Every other kind takes the decoded
            // route below, unchanged.
            let (neighbours, flow) = match &l.source {
                DrawSource::Nested {
                    width,
                    height,
                    background,
                    ..
                } if !l.flow_below.is_empty() => {
                    self.measure_below_flow(l, &tex, *width, *height, *background)
                }
                _ => {
                    // A neighbour is the same footage at another moment, so it
                    // is read the way the frame it sits beside was: a float
                    // plate stays float and a tagged one goes through its
                    // input transform, or the trail is another colour.
                    let (format, colour_space) = match &l.source {
                        DrawSource::Pixels {
                            format,
                            colour_space,
                            ..
                        } => (*format, colour_space.clone()),
                        _ => (lumit_media::decode::PixelFormat::Srgb8, None),
                    };
                    // A frame read last time is the picture made of it then.
                    let upload = |rgba: &[u8], nw, nh| {
                        self.upload_source(rgba, nw, nh, format, &colour_space)
                    };
                    let neighbours: Vec<(i32, wgpu::Texture)> = match self.flow {
                        Some(kept) if !l.neighbours.is_empty() => {
                            kept.borrow_mut().neighbours.pictures(
                                &self.ctx,
                                self.colour_config,
                                &l.neighbours,
                                format,
                                &colour_space,
                                upload,
                            )
                        }
                        _ => l
                            .neighbours
                            .iter()
                            .map(|(offset, rgba, nw, nh)| (*offset, upload(rgba, *nw, *nh)))
                            .collect(),
                    };
                    // The dense motion fields, one per offset a
                    // flow-consuming effect asked for, each uploaded
                    // as its own texture at the size it was filed at: the
                    // layer's raster for Motion blur and Datamosh, which
                    // only bind one that matches it, the measured size for
                    // Denoise. The confidence rides in the .z channel
                    // (FX-19).
                    let flow: Vec<(i32, wgpu::Texture)> = l
                        .flow_fields
                        .iter()
                        .filter(|(_, f)| f.w > 0 && f.h > 0)
                        .map(|(offset, f)| {
                            let (fw, fh) = (f.w as u32, f.h as u32);
                            (
                                *offset,
                                lumit_gpu::fx::common::upload_flow_texels(
                                    &self.ctx, &f.texels, fw, fh,
                                ),
                            )
                        })
                        .collect();
                    (neighbours, flow)
                }
            };
            // The parsed-and-uploaded `.cube` LUTs, 1:1 with the stack's
            // `lut` ops (§3.11); the same load export uses.
            let tables = self.load_tables(&l.colour_tables);
            // The layer inputs — a depth pass, a Light wrap background —
            // resampled to this layer's working raster (w, h), 1:1 with the
            // stack's consuming ops (§3.22, §3.28); the same render export
            // runs.
            let layer_inputs = self.render_layer_inputs(&l.dof_inputs, w, h, l.natural_size, None);
            let mattes = self.render_layer_inputs(&l.mattes, w, h, l.natural_size, None);
            let flare_lens = self.load_flare_lens(&l.flare_lens_files);
            // A stack resolved against a raster wider than the one it is
            // about to run on (a Precomp layer's, under reduced-resolution
            // preview) has its px@comp parameters rescaled to this raster,
            // the same correction the adjustment path applies. `None` —
            // every other kind — resolved at its own decode scale already,
            // so nothing moves.
            let fx_ops = match l.fx_ref_width {
                Some(ref_w) if ref_w > 0.0 => {
                    let mut ops = l.fx.clone();
                    ops.rescale_spatial(w as f32 / ref_w);
                    ops
                }
                _ => l.fx.clone(),
            };
            // The baked pictures a background analysis filed. A roto matte
            // is uploaded at its own raster and `fit_centred` inside the
            // walk takes it to the working one; a plane is resampled to
            // the working raster here instead, by `plane_texture`, because
            // it is at the model's own few-hundred-pixel raster and a
            // crop-and-pad would leave a small square of depth in a sea of
            // nothing. Empty on every layer with no such effect, which
            // costs nothing at all.
            let roto_mattes: Vec<Option<wgpu::Texture>> = l
                .roto_mattes
                .iter()
                .map(|slot| slot.as_ref().map(|m| upload_roto_matte(&self.ctx, m)))
                .collect();
            // The graphs this layer's Node graph effects apply
            // (docs/impl/node-graph-comp.md §2.4): `run_ops` cannot reach
            // the realiser, so it is handed one closure per enabled
            // instance and calls the k-th where the k-th such op would
            // have dispatched a kernel.
            let plans = self.graph_closures(l, w);
            let graphs: Vec<&dyn Fn(wgpu::Texture, u32, u32) -> wgpu::Texture> =
                plans.iter().map(|f| &**f).collect();
            let planes: Vec<Option<wgpu::Texture>> = l
                .planes
                .iter()
                .map(|slot| slot.as_ref().map(|p| self.plane_texture(p, w, h)))
                .collect();
            let late = |slot: usize, points: usize| {
                self.late_clones(l.dof_inputs.get(slot), points, w, h, l.natural_size)
            };
            let side = crate::fxops::Side {
                roto: &roto_mattes,
                planes: &planes,
                late: Some(&late),
            };
            crate::fxops::run_ops(
                self.fx,
                &self.ctx,
                tex,
                w,
                h,
                &fx_ops,
                &neighbours,
                &flow,
                &tables,
                &layer_inputs,
                &flare_lens,
                &mattes,
                &l.mask_paths,
                &l.points_schedules,
                &side,
                &graphs,
                fx_ms.as_mut(),
                // The per-effect cache, for a source the builder
                // could name; one it could not runs as before.
                l.fx_input_key.map(|key| (self.fx_cache, key)),
            )
        };
        // How much its effects grew the picture, which the depth of field
        // reads to place each pixel.
        let grow = (
            tex.width() as f32 / source_w as f32,
            tex.height() as f32 / source_h as f32,
        );
        (tex, grow, model_depth)
    }
}
