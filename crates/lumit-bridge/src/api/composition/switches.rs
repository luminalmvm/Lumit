//! The op one layer switch writes.

use super::*;

/// One layer's op for `switch`, or `None` when the layer already stands at
/// `on` — the one filter-and-build step behind the group broadcast
/// ([`CompositionReference::set_group_switch`]) and the selection batch
/// ([`CompositionReference::set_switch_on_layers`]), so the two cannot come to
/// different ops for the same click.
///
/// The one write it does not build is the adjustment switch coming **off a
/// layer born an adjustment**: that layer needs a picture back, which
/// is a batch of its own, and the selection call assembles it beside this. For
/// every other layer the adjustment switch is the plain flag op, exactly as
/// [`LayerReference::set_adjustment`] writes it.
pub(super) fn layer_switch_op(
    comp: Uuid,
    l: &lumit_core::model::Layer,
    switch: BridgeLayerSwitch,
    on: bool,
) -> Option<lumit_core::ops::Op> {
    use lumit_core::ops::Op;
    let layer = l.id;
    let current = match switch {
        BridgeLayerSwitch::Visible => l.switches.visible,
        BridgeLayerSwitch::Audible => l.switches.audible,
        BridgeLayerSwitch::Locked => l.switches.locked,
        BridgeLayerSwitch::Solo => l.switches.solo,
        BridgeLayerSwitch::ThreeD => l.switches.three_d,
        BridgeLayerSwitch::Fx => l.switches.fx,
        BridgeLayerSwitch::MotionBlur => l.switches.motion_blur,
        BridgeLayerSwitch::Collapse => l.switches.collapse,
        BridgeLayerSwitch::Shy => l.switches.shy,
        BridgeLayerSwitch::AcceptsLights => l.switches.accepts_lights,
        BridgeLayerSwitch::Guide => l.switches.guide,
        BridgeLayerSwitch::Adjustment => l.is_adjustment(),
    };
    if current == on {
        return None;
    }
    Some(match switch {
        BridgeLayerSwitch::Visible => Op::SetLayerVisible {
            comp,
            layer,
            visible: on,
        },
        BridgeLayerSwitch::Audible => Op::SetLayerAudible {
            comp,
            layer,
            audible: on,
        },
        BridgeLayerSwitch::Locked => Op::SetLayerLocked {
            comp,
            layer,
            locked: on,
        },
        BridgeLayerSwitch::Solo => Op::SetLayerSolo {
            comp,
            layer,
            solo: on,
        },
        BridgeLayerSwitch::ThreeD => Op::SetLayerThreeD {
            comp,
            layer,
            three_d: on,
        },
        BridgeLayerSwitch::Fx => Op::SetLayerFx {
            comp,
            layer,
            fx: on,
        },
        BridgeLayerSwitch::MotionBlur => Op::SetLayerMotionBlur {
            comp,
            layer,
            motion_blur: on,
        },
        BridgeLayerSwitch::Collapse => Op::SetLayerCollapse {
            comp,
            layer,
            collapse: on,
        },
        BridgeLayerSwitch::Shy => Op::SetLayerShy {
            comp,
            layer,
            shy: on,
        },
        BridgeLayerSwitch::AcceptsLights => Op::SetLayerAcceptsLights {
            comp,
            layer,
            accepts_lights: on,
        },
        BridgeLayerSwitch::Guide => Op::SetLayerGuide {
            comp,
            layer,
            guide: on,
        },
        BridgeLayerSwitch::Adjustment => Op::SetLayerAdjustment {
            comp,
            layer,
            adjustment: on,
        },
    })
}
