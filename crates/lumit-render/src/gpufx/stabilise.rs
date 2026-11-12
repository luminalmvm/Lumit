//! Stabilise's GPU op: the frame's warp, worked out by the effect from the
//! resolved rows, handed to the warp kernel. A Fill lays the decoded
//! neighbours under it, each through the map the effect worked out for it.

use lumit_core::fx::effects::stabilise::{frame_offset, warp_params, Stabilise as Rows};
use lumit_core::fx::params::Params;
use lumit_core::stabilise::fill::fill_params;
use lumit_gpu::fx::stabilise::{Neighbour, StabiliseFillOp, StabiliseOp};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use super::{AuxKind, AuxSlot, GpuEffect, Tex};

pub(super) struct Stabilise;

impl GpuEffect for Stabilise {
    fn match_name(&self) -> &'static str {
        lumit_core::fx::effects::stabilise::MATCH_NAME
    }

    /// Fill reads the frames either side, which arrive only when it asked for
    /// them; every other setting is handed none.
    fn aux(&self) -> AuxKind {
        AuxKind::Neighbours
    }

    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let Some(wp) = warp_params(p, w, h) else {
            return tex.clone();
        };
        let s = p.float(Rows::DERIVED_PX_SCALE, 1.0).max(1e-6);
        let Some(fill) = fill_params(p, w, h, s, frame_offset(p, w, h)) else {
            return fx.stabilise(
                ctx,
                tex,
                w,
                h,
                &StabiliseOp {
                    map: wp.map,
                    keep: wp.keep,
                    motion: wp.motion,
                    readout: wp.readout,
                },
            );
        };
        // A neighbour that was not decoded, or came at another size, is left
        // out: the fill is thinner there, never wrong.
        let laid: Vec<Neighbour<'_>> = fill
            .neighbours
            .iter()
            .filter_map(|n| {
                let (_, t) = aux.neighbours().iter().find(|(o, _)| *o == n.offset)?;
                (t.width() == w && t.height() == h).then_some((t, n.to_frame, n.motion))
            })
            .collect();
        fx.stabilise_fill(
            ctx,
            tex,
            &laid,
            w,
            h,
            &StabiliseFillOp {
                map: wp.map,
                rect: fill.rect,
                feather: fill.feather,
                motion: wp.motion,
                readout: wp.readout,
            },
        )
    }
}
