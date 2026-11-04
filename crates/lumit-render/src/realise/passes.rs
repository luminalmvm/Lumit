//! A 3D precomp's scene depth for the effects that read it.

use super::Realiser;
use crate::draw::CompLayerDraw;

impl Realiser<'_> {
    /// **Scene depth**: a Precomp's 3D layers as how far each pixel is from its
    /// camera, metres, which is what a lens effect's Depth row reads from a
    /// Precomp. Each layer's picture is made as for colour, so what its effects
    /// cut away is not there. Adjustment and 2D layers draw nothing, so a comp
    /// with no 3D layer is all far away.
    pub fn realise_scene_depth(
        &self,
        n: &crate::draw::NestedInputDraw,
        capture: Option<&mut Option<lumit_gpu::fx::lens_blur::SceneLayers>>,
    ) -> wgpu::Texture {
        // ponytail: 2D layers are still made here and only dropped by the depth
        // pass. Skip them before making them if a heavy 2D layer ever shows.
        // With no 3D layer nothing is made, unless rays want the 2D ones as
        // their backdrop.
        let made = capture.is_some() || n.draws.iter().any(|d| d.three_d);
        let draws: &[CompLayerDraw] = if n.camera.is_some() && made {
            crate::environment::peel(&n.draws).1
        } else {
            &[]
        };
        let metres_per_w = n
            .camera
            .map_or(1.0, |c| (c.zoom / n.px_per_metre.max(1e-6)) as f32);
        let (tw, th) = lumit_gpu::composite::scaled_size(n.width, n.height, self.render_scale);
        self.open_frame(tw, th, draws);
        let out = self.realise_segment(
            n.camera,
            n.width,
            n.height,
            [0.0; 4],
            draws,
            &None,
            None,
            Some(metres_per_w),
            capture,
            None,
            None,
            None,
        );
        self.ctx.end_frame();
        out
    }
}
