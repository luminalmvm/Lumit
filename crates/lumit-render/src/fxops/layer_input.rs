//! A layer another layer reads, set down on the raster of the one reading it.

use super::Tex;
use lumit_gpu::GpuContext;

/// Render one referenced layer alone into the depth input a depth-of-field
/// effect samples (docs/impl/layer-input.md §2). The **one** helper the preview
/// (`GpuViewer`) and export (`Renderer`) paths both call, so the depth pass is
/// byte-identical in the viewport and the file — exactly as
/// `Compositor::motion_blur_average` and the matte "render alone" composite are
/// shared.
///
/// The effect stack runs on the consuming layer's own working raster `(w, h)`
/// (the layer's decoded size, which shrinks under reduced-resolution preview),
/// and the DoF kernel reads the depth at that same pixel grid — so the depth
/// input must be exactly `(w, h)` and aligned with the layer texture. v1 model
/// (documented in docs/08 §3.22): the referenced layer's **source** is
/// resampled to fill `(w, h)` — the depth pass is expected to share the
/// footage's framing (the standard "footage + matching depth pass" workflow),
/// so it is stretched to the working raster and its own transform is not
/// applied. A matte or a depth row is set down by [`place_layer_input`]
/// instead. `linear` is the referenced layer's source in the working linear
/// format, sized `(src_w, src_h)`; each caller uploads/linearises it its own
/// way, as the matte path does, and this helper owns only the resample so it
/// never drifts.
#[allow(clippy::too_many_arguments)]
pub fn render_layer_input(
    compositor: &lumit_gpu::composite::Compositor,
    ctx: &GpuContext,
    w: u32,
    h: u32,
    linear: &Tex,
    premultiplied: bool,
    src_w: f32,
    src_h: f32,
) -> Tex {
    compositor.composite_with_camera(
        ctx,
        w,
        h,
        [0.0, 0.0, 0.0, 0.0],
        &[lumit_gpu::composite::CompositeLayer {
            texture: linear,
            size: (src_w.max(1.0), src_h.max(1.0)),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            // Stretch the source to fill the whole working raster.
            scale: (
                w as f32 / src_w.max(1.0) * 100.0,
                h as f32 / src_h.max(1.0) * 100.0,
            ),
            rotation_deg: 0.0,
            // Full opacity: a depth pass is read as a scalar, never dimmed.
            opacity: 100.0,
            matte: None,
            blend: lumit_gpu::composite::Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
            premultiplied,
        }],
        None,
    )
}

/// [`render_layer_input`]'s sibling for a picture read as a map: set down on
/// the consuming layer's raster `(w, h)` where the Map placement row says,
/// rather than stretched over it. `size` is the referenced layer's own size
/// and `of` the consuming layer's, both px at comp size. Where the picture
/// does not reach, the map reads `place.ground`.
#[allow(clippy::too_many_arguments)]
pub fn place_layer_input(
    compositor: &lumit_gpu::composite::Compositor,
    ctx: &GpuContext,
    w: u32,
    h: u32,
    linear: &Tex,
    premultiplied: bool,
    size: [f32; 2],
    place: &crate::draw::MapPlace,
    of: (f32, f32),
) -> Tex {
    let (sw, sh) = (size[0].max(1.0), size[1].max(1.0));
    let (ow, oh) = (of.0.max(1.0), of.1.max(1.0));
    // The consuming layer's pixels onto its raster, which is smaller under a
    // reduced-resolution preview.
    let raster = lumit_gpu::composite::place_matrix(
        (0.0, 0.0),
        (0.0, 0.0),
        (w as f32 / ow * 100.0, h as f32 / oh * 100.0),
        0.0,
        0.0,
        0.0,
        0.0,
    );
    let onto = place.placed.unwrap_or_else(|| {
        lumit_gpu::composite::place_matrix(
            ((ow - sw) * 0.5, (oh - sh) * 0.5),
            (0.0, 0.0),
            (100.0, 100.0),
            0.0,
            0.0,
            0.0,
            0.0,
        )
    });
    compositor.composite_with_camera(
        ctx,
        w,
        h,
        place.ground,
        &[lumit_gpu::composite::CompositeLayer {
            texture: linear,
            size: (sw, sh),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            scale: (100.0, 100.0),
            rotation_deg: 0.0,
            // Full opacity: a map is read as numbers, never dimmed.
            opacity: 100.0,
            matte: None,
            blend: lumit_gpu::composite::Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: Some(lumit_gpu::composite::concat_place(raster, onto)),
            premultiplied,
        }],
        None,
    )
}
