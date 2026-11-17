//! Shared nodes: one grade node used by several shots
//! ([`lumit_core::grade::shared`]).
//!
//! A shared node is kept with the grade groups, its one effect as the
//! group's Pre list, so it is edited the way a group's grade is: its effect
//! comes from `get_grade_group_effects(node, Pre)` and goes back through the
//! layer's own effect commands, which find it by its id.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use lumit_core::grade::{shared, Shot};

use crate::api::grade_groups::BridgeShotId;
use crate::api::grade_versions::grade_edit;
use crate::api::layer::LayerReference;
use crate::api::project::ProjectReference;
use crate::api::BridgeError;

/// One shared node.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSharedNode {
    pub id: Uuid,
    pub name: String,
    /// The node's effect: its id, for the effect commands, and its match name.
    pub effect: Option<Uuid>,
    pub match_name: Option<String>,
    /// Every shot that uses it.
    pub users: Vec<BridgeShotId>,
    /// The ids its references have in those shots' stacks.
    pub references: Vec<Uuid>,
}

impl ProjectReference {
    /// Every shared node, in the order they were made.
    #[frb(sync)]
    pub fn get_shared_grade_nodes(&self) -> Result<Vec<BridgeSharedNode>, BridgeError> {
        let state = self.state()?;
        let doc = state
            .read()
            .map_err(|_| BridgeError::ReadFailed)?
            .store
            .snapshot();
        Ok(shared::nodes(&doc)
            .map(|g| BridgeSharedNode {
                id: g.id,
                name: g.name.clone(),
                effect: g.pre.first().map(|e| e.id),
                match_name: g.pre.first().map(|e| e.effect.match_name.clone()),
                users: shared::users(&doc, g.id)
                    .into_iter()
                    .map(|s| BridgeShotId {
                        comp: s.comp,
                        layer: s.layer,
                        clip: s.clip,
                    })
                    .collect(),
                references: shared::references(&doc, g.id),
            })
            .collect())
    }

    /// Stop sharing node `node`: each shot that uses it gets its own copy
    /// where it stood, and the shared node goes. One undo step.
    #[frb(sync)]
    pub fn unshare_grade_node(&self, node: Uuid) -> Result<(), BridgeError> {
        grade_edit(&*self.state()?, "Unshare node", |doc| {
            Ok(((), Some(shared::unshare(doc, node)?)))
        })
    }
}

impl LayerReference {
    /// Share node `node` of this shot with `shots`: it moves into a new
    /// shared node, this shot keeps it where it was, and each of `shots`
    /// gets it after its last grade node. One undo step; answers the shared
    /// node's id. Rename it as a group is renamed.
    #[frb(sync)]
    pub fn share_grade_node(
        &self,
        clip: Option<Uuid>,
        node: Uuid,
        shots: Vec<BridgeShotId>,
    ) -> Result<Uuid, BridgeError> {
        let from = self.shot(clip);
        let shots: Vec<Shot> = shots
            .into_iter()
            .map(|s| Shot {
                comp: s.comp,
                layer: s.layer,
                clip: s.clip,
            })
            .collect();
        grade_edit(&*self.project()?, "Share node", |doc| {
            let (id, op) = shared::share(doc, from, node, &shots)?;
            Ok((id, Some(op)))
        })
    }

    /// Shared node `node` in this shot, right after effect `after`, or after
    /// its last grade node. A shot that already uses it keeps the one it has.
    /// One undo step; answers the reference's id in the stack.
    #[frb(sync)]
    pub fn add_shared_grade_node(
        &self,
        clip: Option<Uuid>,
        node: Uuid,
        after: Option<Uuid>,
    ) -> Result<Uuid, BridgeError> {
        let shot = self.shot(clip);
        grade_edit(&*self.project()?, "Edit effects", |doc| {
            shared::add_ref(doc, shot, node, after)
        })
    }
}
