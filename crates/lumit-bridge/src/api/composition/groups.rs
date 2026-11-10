//! A comp's layer groups as the Timeline's read model holds them.

use uuid::Uuid;

use super::BridgeLayerGroup;

/// The comp's groups as the Timeline draws them — resolved once per read
/// model, never per row.
pub(super) fn read_groups(comp: &lumit_core::model::Composition) -> Vec<BridgeLayerGroup> {
    let stack: Vec<Uuid> = comp.layers.iter().map(|l| l.id).collect();
    comp.groups
        .iter()
        .map(|g| {
            let members = lumit_core::group::drawn_members(&stack, g);
            let held: Vec<&lumit_core::model::Layer> = comp
                .layers
                .iter()
                .filter(|l| members.contains(&l.id))
                .collect();
            // An empty group answers a zero span and switch faces that are all
            // on: it draws no row at all, so nothing reads either, and "all of
            // nothing is on" is the honest answer to an `all()` over none.
            BridgeLayerGroup {
                id: g.id,
                name: g.name.clone(),
                label: g.label,
                in_frame: held
                    .iter()
                    .map(|l| comp.frame_rate.frame_at(l.in_point))
                    .min()
                    .unwrap_or(0),
                out_frame: held
                    .iter()
                    .map(|l| comp.frame_rate.frame_at(l.out_point))
                    .max()
                    .unwrap_or(0),
                visible: held.iter().all(|l| l.switches.visible),
                audible: held.iter().all(|l| l.switches.audible),
                solo: held.iter().all(|l| l.switches.solo),
                locked: held.iter().all(|l| l.switches.locked),
                members,
                // Resolve time is comp time: a group has no start
                // offset, so the listing's offset is zero.
                effects: g
                    .effects
                    .iter()
                    .map(|e| {
                        crate::api::effect::read_instance_info(
                            e,
                            lumit_core::time::Rational::ZERO,
                            false,
                        )
                    })
                    .collect(),
            }
        })
        .collect()
}
