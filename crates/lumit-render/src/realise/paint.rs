//! A Precomp layer's paint strokes.

use super::Realiser;

impl Realiser<'_> {
    /// Stamp a Precomp layer's paint strokes into its realised picture.
    ///
    /// Every other kind of layer is painted on the CPU, in the layer's own
    /// 8-bit sRGB raster, before it is uploaded — a Precomp has no such
    /// raster, because its picture is made on the graphics card. So the
    /// picture comes back the way an export reads it (the neutral display
    /// encode, never the Viewer's exposure), goes through the *same*
    /// `apply_strokes` every other layer uses, and is uploaded again. One
    /// rasteriser, one set of rules, and a stroke that lands identically
    /// wherever it is painted.
    ///
    /// ponytail: an 8-bit round trip per painted Precomp per frame — the
    /// nested picture is quantised to the depth every footage layer is
    /// painted at anyway, and read back synchronously (in the browser build,
    /// an earlier frame's copy). A GPU stamping pass (docs/impl/paint.md, "Not
    /// built") is the upgrade if a painted Precomp ever shows in a profile.
    /// Costs nothing on the Precomp layers that carry no paint.
    pub(super) fn paint_over(
        &self,
        tex: wgpu::Texture,
        natural_w: f64,
        natural_h: f64,
        strokes: &[lumit_core::paint::PaintStroke],
        t: f64,
    ) -> wgpu::Texture {
        if strokes.is_empty() {
            return tex;
        }
        let (w, h) = (tex.width(), tex.height());
        let display = self
            .engine
            .display(&self.ctx, &tex, lumit_gpu::DisplayParams::NEUTRAL);
        let read = lumit_gpu::device::read_texture8(&self.ctx, self.engine, "paint", &display);
        let Ok(mut rgba) = read else {
            // The read failed (a lost device, a surface gone). The unpainted
            // picture is the calm answer; engine crates do not panic.
            return tex;
        };
        // Strokes go on straight colour, as on every other layer's raster, and
        // the picture goes back premultiplied, as the rest of a comp's is.
        for px in rgba.chunks_exact_mut(4).filter(|p| p[3] > 0 && p[3] < 255) {
            let a = f32::from(px[3]) / 255.0;
            for c in &mut px[..3] {
                *c = lumit_core::pixels::srgb_encode(lumit_core::pixels::srgb_decode(*c) / a);
            }
        }
        lumit_core::paint::apply_strokes(&mut rgba, w, h, natural_w, natural_h, strokes, t);
        let src = self.engine.upload_srgb8(&self.ctx, &rgba, w, h);
        let linear = self.engine.linearise(&self.ctx, &src);
        crate::fxops::layer_input::render_layer_input(
            self.compositor,
            &self.ctx,
            w,
            h,
            &linear,
            false,
            w as f32,
            h as f32,
        )
    }
}
