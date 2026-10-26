//! Grid's Lines on the GPU: the CPU reference's own setup, carried across to
//! the kernel field for field.

use lumit_core::fx::effects::grid::Grid;
use lumit_core::fx::effects::grid_lines::{GridLines, LineAxis};
use lumit_gpu::fx::grid_lines::{GridLinesOp, LineAxisOp};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

pub(crate) fn draw(
    fx: &FxEngine,
    ctx: &GpuContext,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
    inst: &Grid,
    px_scale: f32,
) -> wgpu::Texture {
    let g = GridLines::of(inst, px_scale);
    let axis = |a: LineAxis| LineAxisOp {
        cell: a.cell,
        origin: a.origin,
        half: a.half,
        reach: a.reach,
        last: a.last,
        middle: a.middle,
    };
    fx.grid_lines(
        ctx,
        tex,
        w,
        h,
        &GridLinesOp {
            centre: g.centre,
            axes: g.axes,
            focal: g.focal,
            distance: g.distance,
            u: axis(g.u),
            v: axis(g.v),
            major_every: g.major_every,
            major_width: g.major_width,
            colour: g.colour,
            mix: g.mix,
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::fx::params::Params;
    use lumit_core::fx::registry::EffectMetadata;

    /// The kernel lands the lines where the CPU reference draws them, tipped,
    /// turned and rolled, for both ways of sizing a cell.
    #[test]
    fn lines_match_the_cpu_reference() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            return;
        };
        let (w, h) = (160u32, 96u32);
        let base: Vec<f32> = [0.2, 0.2, 0.2, 1.0].repeat((w * h) as usize);
        let tex = lumit_gpu::fx::common::upload_linear_f32(&ctx, &base, w, h);
        for size_from in [0, 1] {
            let inst = Grid {
                kind: 1,
                size_from,
                boxes_across: 9,
                boxes_down: 5,
                grid_size_x: 140.0,
                grid_size_y: 84.0,
                position_x: 70.0,
                position_y: 44.0,
                corner_x: 81.0,
                corner_y: 53.0,
                line_width: 1.5,
                latitude: 35.0,
                swing: 12.0,
                roll: -8.0,
                colour: [0.8, 0.4, 0.9, 1.0],
                ..Grid::read(Params::EMPTY)
            };
            let mut cpu = base.clone();
            GridLines::of(&inst, 1.0).draw(&mut cpu, w, h);
            assert_ne!(cpu, base, "the lines drew nothing");
            let out = draw(ctx.fx(), &ctx, &tex, w, h, &inst, 1.0);
            let gpu =
                lumit_gpu::fx::common::readback_linear_f32(&ctx, &out, w, h).expect("readback");
            let worst = cpu
                .iter()
                .zip(&gpu)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 2e-2, "size from {size_from}: worst |Δ| {worst}");
        }
    }
}
