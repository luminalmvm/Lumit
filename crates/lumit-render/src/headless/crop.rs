//! Cropping a finished composite to a region of interest.

/// Copy a sub-rectangle out of a finished composite, so a region of
/// interest returns a region-sized texture even on the frames where the
/// realiser had to composite the whole thing.
///
/// A straight texel copy, not a resample: the crop cannot change a single
/// pixel of what it keeps, which is what makes "with a region" and "without
/// one, then cropped" the same picture rather than nearly the same one.
pub(super) fn crop_texture(
    ctx: &lumit_gpu::GpuContext,
    src: &wgpu::Texture,
    region: lumit_gpu::composite::Region,
    scale: f32,
    comp: (u32, u32),
) -> wgpu::Texture {
    let (mut tw, mut th) = region.target_size(scale);
    tw = tw.min(src.width());
    th = th.min(src.height());
    // Where the window starts on the raster actually rendered. The ratio is
    // taken from that raster rather than from the render scale, so however
    // `scaled_size` rounded, the copy stays inside the source.
    let sx = (region.x * src.width() as f32 / comp.0.max(1) as f32).round() as u32;
    let sy = (region.y * src.height() as f32 / comp.1.max(1) as f32).round() as u32;
    let sx = sx.min(src.width().saturating_sub(tw));
    let sy = sy.min(src.height().saturating_sub(th));
    let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("comp-frame-cropped"),
        size: wgpu::Extent3d {
            width: tw,
            height: th,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: src.format(),
        usage: src.usage(),
        view_formats: &[],
    });
    let mut enc = ctx.encoder("roi-crop");
    enc.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: src,
            mip_level: 0,
            origin: wgpu::Origin3d { x: sx, y: sy, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &out,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: tw,
            height: th,
            depth_or_array_layers: 1,
        },
    );
    drop(enc);
    out
}
