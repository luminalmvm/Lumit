//! Opening a frame: reserving its video memory first.

use super::Realiser;
use crate::draw::{CompLayerDraw, DrawSource};

impl Realiser<'_> {
    /// Reserve this frame's video memory and open its command batch, stepping
    /// down the degradation ladder (docs/13 §4) if the governor says no.
    ///
    /// # In plain terms
    ///
    /// The two rungs the renderer owns are here, in order and cheapest first:
    /// stop filling the intermediate cache, then give the cold half of it back.
    /// Both are invisible — the picture that comes out is identical, only made
    /// again rather than read back — which is exactly why they are the rungs
    /// that may be taken on a memory reading. The rungs that *are* visible (the
    /// preview resolution tier, tiling, the flow swap, the calm banner) change
    /// what is drawn, so they belong to the caller, which reads the denial
    /// count and steps down deliberately; export never takes them at all.
    ///
    /// A frame that is still refused after both is **rendered anyway**. There
    /// is nothing honest to return instead — a texture cannot be refused half
    /// way through a pass, and a black frame is a worse answer than a slow one
    /// — so what the refusal buys is a truthful readout and a governor that
    /// says no earlier next time, not an abort. Nested frames take this path
    /// too and reserve nothing: they are already inside their parent's
    /// reservation, and counting them again would refuse a frame that fits.
    pub(super) fn open_frame(&self, tw: u32, th: u32, layers: &[CompLayerDraw]) {
        let estimate = self.frame_estimate(tw, th, layers);
        if self.ctx.try_begin_frame(estimate).is_ok() {
            return;
        }
        // Rung one is already taken and takes itself back: the effect walk
        // reads the same pressure and stops filing while it lasts
        // ([`crate::fxops::run_ops`]), so there is no flag here to set and
        // later remember to clear. Rung two is this.
        self.fx_cache.borrow_mut().trim_under_pressure();
        self.let_go_of_kept();
        if self.ctx.try_begin_frame(estimate).is_err() {
            self.ctx.begin_frame();
        }
    }

    /// What this frame is expected to hold on the card at its **peak** — not
    /// its total, which is a far larger and far less useful number.
    ///
    /// Layers are composited one at a time into one accumulator and the effect
    /// walk ping-pongs through the frame's texture pool, so a hundred-layer
    /// comp peaks at a handful of pictures rather than a hundred. What does
    /// grow with the project is the layers that stage through a comp-sized
    /// intermediate of their own — an adjustment layer, which needs everything
    /// below it composited first, and a motion-blurred one, which holds an
    /// accumulator across its shutter. They are the same two the region
    /// window steps aside for ([`region_is_safe`]), and for the same reason.
    fn frame_estimate(&self, tw: u32, th: u32, layers: &[CompLayerDraw]) -> u64 {
        /// The accumulator, the layer being drawn, the two the effect walk
        /// passes between, and headroom for the matte or plate an effect reads
        /// beside them.
        const FRAME_PEAK_TEXTURES: u64 = 6;
        let staged = layers
            .iter()
            .filter(|l| {
                matches!(l.source, DrawSource::Adjust | DrawSource::Haze(_)) || !l.mb.is_empty()
            })
            .count() as u64;
        lumit_gpu::texture_bytes(self.ctx.working(), tw, th)
            .saturating_mul(FRAME_PEAK_TEXTURES.saturating_add(staged))
    }
}
