//! What a comp's draws mix in.

use crate::draw::CompLayerDraw;
use lumit_core::model::blending::CompBlending;

/// Mark each of a comp's draws with what the comp mixes its layers in. A
/// collapsed Precomp's draws are marked again by the comp they land in, so
/// they mix as it does.
pub(super) fn stamp(draws: &mut [CompLayerDraw], comp: &lumit_core::model::Composition) {
    let display = comp.blending == CompBlending::Display;
    for d in draws {
        d.display_blend = display;
    }
}
