//! Full-frame copies of a picture into a composite's target, and the comp
//! that blends in display: its target holds display values while its layers
//! go in, and is drawn back to linear once they are, so what leaves the
//! compositor is linear either way.

use super::{Compositor, LayerUniform, PIECE_CORNERS};
use crate::GpuContext;

/// What a full-frame draw through `fs_seed` does to each pixel it copies.
#[derive(Clone, Copy)]
pub(super) enum Seed {
    Copy,
    ToDisplay,
    ToLinear,
}

/// A linear premultiplied colour as display values, for the clear of a
/// target that holds them.
pub(super) fn display_colour(c: [f64; 4]) -> [f64; 4] {
    if c[3] <= 0.0 {
        return c;
    }
    let encode = |v: f64| {
        let v = v / c[3];
        let e = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        e * c[3]
    };
    [encode(c[0]), encode(c[1]), encode(c[2]), c[3]]
}

impl Compositor {
    /// `from` drawn over the whole of `attachment`, which it replaces, doing
    /// `seed` to each pixel. `textureLoad` at the fragment's own pixel keeps a
    /// copy exact. The buffer and bind group come back to be kept until the
    /// commands are submitted.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn seed_draw(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: Option<&wgpu::RenderPipeline>,
        from: &wgpu::Texture,
        attachment: wgpu::RenderPassColorAttachment<'_>,
        (width, height): (u32, u32),
        seed: Seed,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let mode = match seed {
            Seed::Copy => 0.0,
            Seed::ToDisplay => 1.0,
            Seed::ToLinear => -1.0,
        };
        let uniform = LayerUniform {
            matrix: self.full_frame_matrix(width, height, None),
            params: [1.0, 0.0, 0.0, 0.0],
            target: [from.width() as f32, from.height() as f32, -1.0, 0.0],
            cut: [0.0, 0.0, mode, 0.0],
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
                        &from.create_view(&Default::default()),
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
            color_attachments: &[Some(attachment)],
            ..Default::default()
        });
        if let Some(pipeline) = pipeline {
            rpass.set_pipeline(pipeline);
        }
        rpass.set_bind_group(0, &bind, &[0]);
        rpass.draw(0..6, 0..1);
        drop(rpass);
        (buffer, bind)
    }

    /// `frame`, which holds display values, drawn back to linear in a new
    /// texture of the same size.
    pub(super) fn to_linear(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        (width, height): (u32, u32),
    ) -> (wgpu::Texture, (wgpu::Buffer, wgpu::BindGroup)) {
        let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("comp-frame"),
            size: frame.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: frame.format(),
            usage: frame.usage(),
            view_formats: &[],
        });
        let view = out.create_view(&Default::default());
        let pipelines = self.pipelines(ctx, 1, None);
        let keep = self.seed_draw(
            ctx,
            encoder,
            pipelines.seed.as_ref(),
            frame,
            wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            },
            (width, height),
            Seed::ToLinear,
        );
        (out, keep)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::{Blend, CompositeLayer};

    fn encode(v: f64) -> f64 {
        if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    }

    fn decode(e: f64) -> f64 {
        if e <= 0.040_45 {
            e / 12.92
        } else {
            ((e + 0.055) / 1.055).powf(2.4)
        }
    }

    /// White at 38 % over a dark ground reads as a browser shows it in a comp
    /// that blends in display, and as it always did in one that does not. A
    /// later stage seeded with that picture, as above an adjustment layer,
    /// hands it back unchanged.
    #[test]
    fn display_blending_mixes_as_a_browser_does() {
        let Some(ctx) = crate::test_support::lease() else {
            crate::no_adapter();
            return;
        };
        let colour = ctx.colour();
        let compositor = ctx.compositor();
        let white = colour.linearise(&ctx, &colour.upload_srgb8(&ctx, &[255u8; 64], 4, 4));
        let layer = CompositeLayer {
            texture: &white,
            size: (4.0, 4.0),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            opacity: 38.0,
            matte: None,
            blend: Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
            premultiplied: false,
        };
        let ground = 16.0 / 255.0;
        let bg = decode(ground);
        let shown = |tex: &wgpu::Texture| {
            let out = colour.display(&ctx, tex, crate::DisplayParams::NEUTRAL);
            i16::from(colour.readback8(&ctx, &out).unwrap()[0])
        };
        let linear_expect = (encode(0.38 + 0.62 * bg) * 255.0).round() as i16;
        let display_expect = ((0.38 + 0.62 * ground) * 255.0).round() as i16;
        for (display, expect) in [(false, linear_expect), (true, display_expect)] {
            let made = compositor.composite_joined(
                &ctx,
                4,
                4,
                [bg, bg, bg, 1.0],
                std::slice::from_ref(&layer),
                None,
                None,
                1.0,
                1,
                None,
                &[],
                &[],
                display,
            );
            let got = shown(&made);
            assert!(
                (got - expect).abs() <= 1,
                "display {display}: {got} vs {expect}"
            );
            let again = compositor.composite_joined(
                &ctx,
                4,
                4,
                [0.0; 4],
                &[],
                None,
                Some(&made),
                1.0,
                1,
                None,
                &[],
                &[],
                display,
            );
            assert!(
                (shown(&again) - got).abs() <= 1,
                "display {display}: the seed moved"
            );
        }
    }
}
