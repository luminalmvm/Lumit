//! One pass of the shared gaussian (`fx_blur.wgsl`), dispatched in tiles of
//! rows or columns rather than the usual 8×8 squares.

use super::{blur::BlurParams, FxEngine};
use crate::GpuContext;

/// A workgroup of `fx_blur.wgsl`: `TILE` pixels along the pass on each of
/// `LINES` lines across it, as the kernel's constants of the same names.
const TILE: u32 = 128;
const LINES: u32 = 4;

impl FxEngine {
    /// Blur `src` into `dst` along `p.dir`, blending against `orig`, with
    /// `matte` bound as [`Self::dispatch_matted`] binds it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn gaussian_pass(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        orig: &wgpu::Texture,
        matte: Option<&wgpu::Texture>,
        dst: &wgpu::Texture,
        w: u32,
        h: u32,
        p: &BlurParams,
    ) {
        let groups = if p.dir[0] > 0.5 {
            [w.div_ceil(TILE), h.div_ceil(LINES)]
        } else {
            [w.div_ceil(LINES), h.div_ceil(TILE)]
        };
        let params = bytemuck::bytes_of(p);
        self.dispatch_groups(ctx, &self.blur, src, orig, matte, dst, groups, params);
    }
}
