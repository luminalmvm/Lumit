//! The frame key's share of an effect that shows its own input at other
//! moments (`lumit_core::fx::moments`).

use std::sync::Arc;

use lumit_core::model::{Composition, Document};
use uuid::Uuid;

use super::{feed_f64, feed_layer, Quality, SourceStamper};

/// **The moments an effect shows its own input at** (Timewarp,
/// `lumit_core::fx::moments`): the layer built again at each, or on an
/// adjustment layer every layer below it, fed as they are at that moment. The
/// moments are the ones the builder renders, from the same code.
#[allow(clippy::too_many_arguments)]
pub(crate) fn feed_own_moments(
    h: &mut blake3::Hasher,
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &lumit_core::model::Layer,
    t: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
) -> Option<()> {
    let times = lumit_core::fx::moments::own_moment_comp_times(comp, layer, t);
    if times.is_empty() {
        return Some(());
    }
    h.update(b"own-moments/");
    let below = comp
        .layers
        .iter()
        .position(|l| l.id == layer.id)
        .map_or(&[][..], |i| &comp.layers[i + 1..]);
    for tau in times {
        feed_f64(h, tau);
        // The layer itself is built at every moment, in its span or not. The
        // layers below an adjustment are drawn as the comp draws them there,
        // their own moments with them.
        if !layer.is_adjustment() {
            let lt = lumit_core::time::layer_time(tau, layer.start_offset.0);
            feed_layer(h, doc, comp, layer, tau, lt, quality, stamper, visited)?;
            continue;
        }
        for l in below {
            let live = l.switches.visible
                && !l.audio_only
                && tau >= l.in_point.0.to_f64()
                && tau < l.out_point.0.to_f64();
            h.update(&[u8::from(live)]);
            if live {
                let lt = lumit_core::time::layer_time(tau, l.start_offset.0);
                feed_layer(h, doc, comp, l, tau, lt, quality, stamper, visited)?;
                feed_own_moments(h, doc, comp, l, tau, quality, stamper, visited)?;
            }
        }
    }
    Some(())
}
