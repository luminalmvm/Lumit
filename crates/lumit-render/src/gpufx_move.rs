//! Move's GPU pass: Shake's averaging kernel, then an opacity.

use lumit_core::fx::effects::move_transition::MoveTransition;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::stylise::{ShakeMbOp, ShakeMbTap, TransformOp, SHAKE_MB_SAMPLES};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct MoveFx;

impl GpuEffect for MoveFx {
    fn match_name(&self) -> &'static str {
        "move"
    }

    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &wgpu::Texture,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> wgpu::Texture {
        let m = MoveTransition::read(p).packed(w, h);
        if m.identity {
            return tex.clone();
        }
        let mut taps = [ShakeMbTap {
            m: [1.0, 0.0, 0.0, 1.0],
            off: [0.0, 0.0],
            channels: 7,
        }; SHAKE_MB_SAMPLES];
        for (tap, (m, off, channels)) in taps.iter_mut().zip(m.taps) {
            *tap = ShakeMbTap { m, off, channels };
        }
        let op = ShakeMbOp {
            taps,
            count: m.count as u32,
            edge: m.edge,
            crop: [0, 0, w, h],
            mix: m.mix,
        };
        let moved = fx.shake_mb(ctx, tex, w, h, None, &op);
        if m.alpha >= 1.0 {
            return moved;
        }
        let faded = fx.transform(
            ctx,
            &moved,
            w,
            h,
            None,
            &TransformOp {
                m: [1.0, 0.0, 0.0, 1.0],
                off: [0.0, 0.0],
                opacity: m.alpha,
                mix: 1.0,
                edge: 0,
            },
        );
        ctx.recycle(moved);
        faded
    }
}
