//! The generic points draw: a host-built set of points through
//! Particulate's own instanced quad.
//!
//! **In plain terms.** Particulate works its particles out *on the card*,
//! because there can be a million of them and each one is a page of algebra.
//! The **generators** are not like that: a grid is closed-form arithmetic over
//! a few hundred cells, and a scatter's candidates are two hashes each. Working
//! those out on the host and posting the answers costs less than a compute pass
//! would, and it buys something better than speed — the points a generator
//! *draws* are, bit for bit, the points its CPU reference evaluated, so the two
//! cannot drift.
//!
//! What arrives here is [`DrawPoint`]s. What happens to them is that they are
//! laid out in the very stream layout Particulate's compaction writes, and
//! handed to the very pipeline Particulate's quads go through. One rasteriser
//! for the whole family: a disc drawn by Grid is the disc Particulate draws.

use crate::GpuContext;

use super::{particulate::ParticulateParams, particulate::STREAM_WORDS, work_texture, FxEngine};

/// **Which picture-derived field a point is put to a vote against**, in the
/// vertex stage.
///
/// One rejection, two rules, because the two effects that want one differ only
/// in what they read off the pixel under a point: Scatter asks how covered it
/// is, Emit from image asks how bright it is. A refused point is given no size,
/// which draws nothing at all — a disc of no radius covers no pixel — rather
/// than being compacted away, because the picture is the only thing the pass
/// makes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldTest {
    /// Every point stands. Particulate's were decided by the compaction, and a
    /// generator's set is its set.
    None,
    /// **Scatter's**: the alpha under the point, optionally inverted so
    /// the points land where the alpha is *not*.
    Alpha { invert: bool },
    /// **Emit from image's**: the unpremultiplied luminance under the
    /// point, remapped so `threshold` is no chance at all and full white is
    /// every chance.
    Luma { threshold: f32 },
}

impl FieldTest {
    /// The kernel's own mode code — the `FIELD_*` constants of
    /// `fx_particulate.wgsl`, which this enum is the twin of.
    #[must_use]
    pub fn mode(self) -> u32 {
        match self {
            FieldTest::None => 0,
            FieldTest::Alpha { .. } => 1,
            FieldTest::Luma { .. } => 2,
        }
    }
}

/// One point, as the generic draw reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawPoint {
    /// px in the layer's three axes, **unprojected** — the camera is
    /// applied in the vertex stage, as it is for a particle.
    pub position: [f32; 3],
    /// Diameter, px.
    pub size: f32,
    /// Radians.
    pub rotation: f32,
    /// Premultiplied scene-linear RGBA.
    pub colour: [f32; 4],
    /// The point's index in its generator's own ordering — the stream's `id`,
    /// and what Scatter's acceptance die is drawn against in the vertex stage.
    pub id: u32,
    /// Where the capsule runs back to, px in the same three axes. The
    /// **head** for a plain dot, which is what makes a disc a streak of no
    /// length and lets one kernel serve both without a branch.
    pub tail: [f32; 3],
    /// How far the size is stretched across and down, in the point's own
    /// turned frame. `[1, 1]` is unstretched.
    pub stretch: [f32; 2],
}

/// How a sprite sits on its point, the twin of
/// `lumit_core::fx::points::SpriteFit`. The default is what Particulate
/// stamps: a square of the point's size with the whole picture across it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteFit {
    /// The stamp's width and height, as multiples of the point's size.
    pub unit: [f32; 2],
    /// The place on the stamp that sits on the point, 0 to 1 from its top
    /// left corner. The stamp turns about it.
    pub anchor: [f32; 2],
    /// How many times the picture's width and height the stamp is. Above 1
    /// leaves the stamp empty round the picture, below 1 cuts the picture off.
    pub uv: [f32; 2],
    /// Corner radius, px on the stamp of a point whose size is 100.
    pub corner: f32,
}

impl Default for SpriteFit {
    fn default() -> Self {
        Self {
            unit: [1.0; 2],
            anchor: [0.5; 2],
            uv: [1.0; 2],
            corner: 0.0,
        }
    }
}

/// One generic points draw.
#[derive(Debug, Clone, Copy)]
pub struct PointsDrawOp<'a> {
    pub points: &'a [DrawPoint],
    /// Disc edge softness, `0..=1`.
    pub feather: f32,
    /// The host Mix, `0..=1`, folded into the point's own coverage.
    pub mix: f32,
    /// The composition's camera, already scaled to this raster, or
    /// `None` on a 2D layer — where it is not the identity matrix but no
    /// matrix at all, so the positions' bits are left alone.
    pub projection: Option<[[f32; 4]; 3]>,
    /// **The rejection**: which picture-derived field each point
    /// is put to a vote against, and by what rule. The test happens in the
    /// vertex stage because that is the only place a host-built point set can
    /// meet a picture that exists only on the card.
    pub field: FieldTest,
    /// The seed the acceptance die is drawn from; unread under
    /// [`FieldTest::None`].
    pub seed: u32,
    /// Which coverage the quad is filled with — the kernel's own mode codes,
    /// matching `lumit_core::fx::points::RenderMode`: 0 a feathered disc (or a
    /// capsule, when a point's tail is not its head), 1 a **sprite**.
    ///
    /// Sprite mode is Clone to points', and it is the same mode
    /// Particulate's own draw runs — one rasteriser for the whole family, so a
    /// stamp laid by a consumer is the stamp Particulate lays.
    pub mode: u32,
    /// The picture Sprite mode stamps. `None` in every other mode, and an unset
    /// one leaves the mode as the caller declared it: the *host* decides what an
    /// absent source means, because one branch resolved there cannot come to
    /// mean two things in two places.
    pub sprite: Option<&'a wgpu::Texture>,
}

impl FxEngine {
    /// Draw a host-built set of points over a working texture, returning a new
    /// texture of the same size.
    ///
    /// `field` is the picture the rejection reads from — a bound matte's for
    /// Scatter, the Source layer's for Emit from image, or `None` for the
    /// input itself. Unread under [`FieldTest::None`].
    ///
    /// The picture is copied first and the discs drawn over it, so this is an
    /// ordinary "picture in, picture out" pass — and an empty set, a Mix of
    /// nothing, or a field the alpha refused entirely all leave the input
    /// exactly as it arrived.
    pub fn points_draw(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        field: Option<&wgpu::Texture>,
        op: &PointsDrawOp<'_>,
    ) -> wgpu::Texture {
        let out = points_copy(ctx, src, w, h);
        if op.points.is_empty() || op.mix <= 0.0 {
            return out;
        }
        let plain = SpriteFit::default();
        self.points_pass(ctx, src, w, h, field, op, &out, None, &plain);
        out
    }

    /// Several draws over one copy of the picture, laid down in the order
    /// given, each with its own sprite and its own fit. Clone to points' draw:
    /// a run of stamps per layer, and no field test.
    pub fn points_draw_runs(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        runs: &[(PointsDrawOp<'_>, SpriteFit)],
    ) -> wgpu::Texture {
        let out = points_copy(ctx, src, w, h);
        for (op, fit) in runs {
            if op.points.is_empty() || op.mix <= 0.0 {
                continue;
            }
            self.points_pass(ctx, src, w, h, None, op, &out, None, fit);
        }
        out
    }

    /// Which of `op`'s points the field keeps, one answer per point in order.
    ///
    /// The same vertex-stage test [`points_draw`](Self::points_draw) runs, read
    /// back, so a points stream made from the kept ones is the set the draw
    /// shows. Costs one small synchronous readback. A readback that fails
    /// keeps nothing.
    pub fn points_stood(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        field: Option<&wgpu::Texture>,
        op: &PointsDrawOp<'_>,
    ) -> Vec<bool> {
        let count = op.points.len();
        if count == 0 || op.field == FieldTest::None {
            return vec![true; count];
        }
        // A kept point wrote white and a refused one nothing.
        self.points_probe(ctx, src, w, h, field, op)
            .iter()
            .map(|px| px.iter().any(|v| *v > 0.0))
            .collect()
    }

    /// The colour of `field` under each of `op`'s points, premultiplied RGBA,
    /// one per point in order. `None` reads `src`. A point off the picture
    /// reads its nearest edge. Costs one small synchronous readback.
    pub fn points_sampled(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        field: Option<&wgpu::Texture>,
        op: &PointsDrawOp<'_>,
    ) -> Vec<[f32; 4]> {
        let plain = PointsDrawOp {
            field: FieldTest::None,
            ..*op
        };
        self.points_probe(ctx, src, w, h, field, &plain)
    }

    /// One pixel per point, drawn by the vertex stage and read back: white or
    /// nothing under a field test, the field's own colour without one. A
    /// readback that fails answers black for every point.
    fn points_probe(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        field: Option<&wgpu::Texture>,
        op: &PointsDrawOp<'_>,
    ) -> Vec<[f32; 4]> {
        let count = op.points.len();
        if count == 0 {
            return Vec::new();
        }
        // A square-ish target with a pixel per point.
        let pw = ((count as f64).sqrt().ceil() as u32).max(1);
        let ph = (count as u32).div_ceil(pw).max(1);
        let target = work_texture(ctx, pw, ph, "fx-points-stood");
        let plain = SpriteFit::default();
        self.points_pass(ctx, src, w, h, field, op, &target, Some((pw, ph)), &plain);

        let bytes = target.format().block_copy_size(None).unwrap_or(8);
        let padded = (pw * bytes).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-points-stood"),
            size: u64::from(padded) * u64::from(ph),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        // Hand over what the frame batch holds first, the pass above included.
        ctx.flush();
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(ph),
                },
            },
            target.size(),
        );
        ctx.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::Maintain::Wait);
        let mut read = vec![[0.0f32; 4]; count];
        if matches!(rx.recv(), Ok(Ok(()))) {
            let data = slice.get_mapped_range();
            for (i, out) in read.iter_mut().enumerate() {
                let at = (i as u32 / pw * padded + i as u32 % pw * bytes) as usize;
                let Some(px) = data.get(at..at + bytes as usize) else {
                    continue;
                };
                // Whichever of the three working formats the target is.
                for (c, v) in out.iter_mut().enumerate() {
                    *v = match bytes {
                        16 => px
                            .get(c * 4..c * 4 + 4)
                            .and_then(|b| b.try_into().ok())
                            .map_or(0.0, f32::from_le_bytes),
                        8 => px
                            .get(c * 2..c * 2 + 2)
                            .and_then(|b| b.try_into().ok())
                            .map_or(0.0, |b| super::f16_to_f32(u16::from_le_bytes(b))),
                        _ => px.get(c).map_or(0.0, |b| f32::from(*b) / 255.0),
                    };
                }
            }
            drop(data);
            buffer.unmap();
        }
        ctx.recycle(target);
        read
    }

    /// The instanced draw behind both entry points: over `target` as it
    /// stands, or cleared and a pixel per point when `probe` gives its size.
    #[allow(clippy::too_many_arguments)]
    fn points_pass(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        field: Option<&wgpu::Texture>,
        op: &PointsDrawOp<'_>,
        out: &wgpu::Texture,
        probe: Option<(u32, u32)>,
        fit: &SpriteFit,
    ) {
        use wgpu::util::DeviceExt;
        let count = u32::try_from(op.points.len()).unwrap_or(u32::MAX);
        // The stream layout Particulate's compaction writes, filled from the
        // host instead: the regions the draw reads carry the points, and the
        // ones only a data consumer would read stay nought. The strides are
        // the kernel's own `r_*` functions, in words, for a capacity of
        // `count`.
        //
        // ponytail: the whole set is uploaded every frame — 68 bytes a point,
        // well under a megabyte at the default caps. Give the generators a
        // compute pass of their own if a profile ever shows this copy costing
        // more than the closed forms it saves.
        let cap = u64::from(count);
        let mut words = vec![0u32; (cap * STREAM_WORDS) as usize];
        let region = |k: u64| (k * cap) as usize;
        for (i, pt) in op.points.iter().enumerate() {
            for c in 0..3 {
                words[region(0) + i * 3 + c] = pt.position[c].to_bits();
                words[region(14) + i * 3 + c] = pt.tail[c].to_bits();
            }
            // A probe asks who stood, whatever size they are drawn at.
            let size = if probe.is_some() { 1.0 } else { pt.size };
            words[region(8) + i] = size.to_bits();
            words[region(9) + i] = pt.rotation.to_bits();
            // Half precision, as particulate.md §4 declares the colour region.
            let half = |v: f32| u32::from(half::f16::from_f32(v).to_bits());
            words[region(10) + i * 2] = half(pt.colour[0]) | (half(pt.colour[1]) << 16);
            words[region(10) + i * 2 + 1] = half(pt.colour[2]) | (half(pt.colour[3]) << 16);
            words[region(12) + i * 2] = pt.id;
            // Stored as the amount past 1, so a region nobody wrote reads as
            // unstretched. Particulate's own pass never writes it.
            for c in 0..2 {
                words[region(17) + i * 2 + c] = (pt.stretch[c] - 1.0).to_bits();
            }
        }
        let proj = op.projection.unwrap_or([[0.0; 4]; 3]);
        let mut u: ParticulateParams = bytemuck::Zeroable::zeroed();
        u.cap = count;
        u.seed = op.seed;
        u.feather = op.feather;
        u.mix = op.mix;
        u.mode = if probe.is_some() { 0 } else { op.mode };
        (u.probe_w, u.probe_h) = probe.unwrap_or((0, 0));
        u.target_w = w as f32;
        u.target_h = h as f32;
        u.sprite_w = op.sprite.map_or(1.0, |s| s.width() as f32);
        u.sprite_h = op.sprite.map_or(1.0, |s| s.height() as f32);
        u.proj0 = proj[0];
        u.proj1 = proj[1];
        u.proj2 = proj[2];
        u.project = u32::from(op.projection.is_some());
        u.field_mode = op.field.mode();
        u.field_invert = u32::from(matches!(op.field, FieldTest::Alpha { invert: true }));
        u.field_threshold = match op.field {
            FieldTest::Luma { threshold } => threshold.clamp(0.0, 1.0),
            _ => 0.0,
        };
        // Each as the amount past the plain square, so that nought is what
        // Particulate's own draw gets by never writing them.
        u.stamp_unit = [fit.unit[0] - 1.0, fit.unit[1] - 1.0];
        u.stamp_anchor = [fit.anchor[0] - 0.5, fit.anchor[1] - 0.5];
        u.stamp_uv = [fit.uv[0] - 1.0, fit.uv[1] - 1.0];
        u.stamp_corner = fit.corner;
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-points-params"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let stream = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-points-stream"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // The sprite Clone to points stamps, or the input picture in that slot
        // when nothing stamps at all — Disc mode never samples it. Beside it,
        // the rejection's own field: a bound matte's or a Source layer's
        // picture, or this effect's input when the row is unset.
        let view = op.sprite.unwrap_or(src).create_view(&Default::default());
        let field_view = field.unwrap_or(src).create_view(&Default::default());
        let draw_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-points-draw-bind"),
            layout: &self.particulate_draw_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: stream.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&field_view),
                },
            ],
        });
        let empty = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-points-empty"),
            layout: &self.particulate_empty_layout,
            entries: &[],
        });
        let target = out.create_view(&Default::default());
        let mut enc = ctx.encoder("fx-points-draw");
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx-points-draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The picture is already there, copied in by the
                        // caller. A probe starts from black.
                        load: match probe {
                            Some(_) => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            None => wgpu::LoadOp::Load,
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rp.set_pipeline(&self.particulate_draw);
            rp.set_bind_group(0, &empty, &[]);
            rp.set_bind_group(1, &draw_bind, &[]);
            rp.draw(0..6, 0..count);
        }
        drop(enc);
    }
}

/// A copy of `src` for the points to be drawn over.
fn points_copy(ctx: &GpuContext, src: &wgpu::Texture, w: u32, h: u32) -> wgpu::Texture {
    let out = work_texture(ctx, w, h, "fx-points-out");
    let mut enc = ctx.encoder("fx-points-copy");
    enc.copy_texture_to_texture(
        src.as_image_copy(),
        out.as_image_copy(),
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    drop(enc);
    out
}
