//! Grade groups: shots sharing a grade laid on before theirs and one after
//! ([`lumit_core::grade::groups`]).
//!
//! A group's two grades are effect lists like any other, so their effects are
//! handed out as ordinary handles and edited through the layer's own effect
//! commands: values, keys, bypass, reorder, remove and the drag preview all
//! find a group's effect by its id.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use lumit_core::fx::effects::grade_group::Part;
use lumit_core::grade::{groups, GradeError, Shot};

use crate::api::effect::BridgeEffectInstance;
use crate::api::grade_versions::grade_edit;
use crate::api::layer::LayerReference;
use crate::api::project::ProjectReference;
use crate::api::BridgeError;

/// One grade group.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGradeGroup {
    pub id: Uuid,
    pub name: String,
    /// How many shots are in it, in every comp.
    pub members: u32,
}

/// A shot, named from outside any one layer: a layer, or with `clip` one clip
/// on a Sequence layer.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeShotId {
    pub comp: Uuid,
    pub layer: Uuid,
    pub clip: Option<Uuid>,
}

/// Which of a group's two grades: the one before each member's own, or after.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeGradePart {
    Pre,
    Post,
}

impl From<BridgeGradePart> for Part {
    fn from(p: BridgeGradePart) -> Self {
        match p {
            BridgeGradePart::Pre => Part::Pre,
            BridgeGradePart::Post => Part::Post,
        }
    }
}

impl ProjectReference {
    /// Every grade group, in the order they were made. Shared nodes are
    /// kept with them but listed apart ([`Self::get_shared_grade_nodes`]).
    #[frb(sync)]
    pub fn get_grade_groups(&self) -> Result<Vec<BridgeGradeGroup>, BridgeError> {
        let state = self.state()?;
        let doc = state
            .read()
            .map_err(|_| BridgeError::ReadFailed)?
            .store
            .snapshot();
        Ok(doc
            .grade_groups
            .iter()
            .filter(|g| !g.shared)
            .map(|g| BridgeGradeGroup {
                id: g.id,
                name: g.name.clone(),
                members: u32::try_from(groups::members(&doc, g.id).len()).unwrap_or(u32::MAX),
            })
            .collect())
    }

    /// A new group with nothing in its grades yet, and `shots` in it, each
    /// leaving any group it was in. Answers its id.
    #[frb(sync)]
    pub fn create_grade_group(&self, shots: Vec<BridgeShotId>) -> Result<Uuid, BridgeError> {
        let shots: Vec<Shot> = shots
            .into_iter()
            .map(|s| Shot {
                comp: s.comp,
                layer: s.layer,
                clip: s.clip,
            })
            .collect();
        grade_edit(&*self.state()?, "New grade group", |doc| {
            let (id, op) = groups::create(doc, &shots)?;
            Ok((id, Some(op)))
        })
    }

    #[frb(sync)]
    pub fn rename_grade_group(&self, group: Uuid, name: String) -> Result<(), BridgeError> {
        grade_edit(&*self.state()?, "Rename grade group", |doc| {
            Ok(((), groups::rename(doc, group, &name)?))
        })
    }

    /// Delete a group, and its place in every member's stack.
    #[frb(sync)]
    pub fn delete_grade_group(&self, group: Uuid) -> Result<(), BridgeError> {
        grade_edit(&*self.state()?, "Delete grade group", |doc| {
            Ok(((), Some(groups::delete(doc, group)?)))
        })
    }
}

impl LayerReference {
    /// The group the shot is in, if any.
    #[frb(sync)]
    pub fn get_grade_group(&self, clip: Option<Uuid>) -> Result<Option<Uuid>, BridgeError> {
        let doc = self.document()?;
        let (stack, _) = lumit_core::grade::read(&doc, self.shot(clip))
            .map_err(crate::api::grade_versions::refused)?;
        Ok(groups::group_in(stack))
    }

    /// Put the shot in `group`, move it there from another, or with `None`
    /// take it out of any.
    #[frb(sync)]
    pub fn set_grade_group(
        &self,
        clip: Option<Uuid>,
        group: Option<Uuid>,
    ) -> Result<(), BridgeError> {
        let shot = self.shot(clip);
        let name = if group.is_some() {
            "Join grade group"
        } else {
            "Leave grade group"
        };
        grade_edit(&*self.project()?, name, |doc| {
            Ok(((), groups::set_group(doc, shot, group)?))
        })
    }

    /// One of a group's grades as staged copies, edited through this layer's
    /// effect commands like its own. Their keys are on the comp's clock.
    #[frb(sync)]
    pub fn get_grade_group_effects(
        &self,
        group: Uuid,
        part: BridgeGradePart,
    ) -> Result<Vec<BridgeEffectInstance>, BridgeError> {
        let doc = self.document()?;
        let g = groups::group(&doc, group).ok_or(BridgeError::InvalidItem)?;
        Ok(g.part(part.into())
            .iter()
            .map(|e| BridgeEffectInstance::new(e.clone(), lumit_core::time::Rational::ZERO))
            .collect())
    }

    /// Add the effect called `name` to the end of one of a group's grades. A
    /// driver, a graph-only effect and a group's own reference are refused.
    #[frb(sync)]
    pub fn add_grade_group_effect(
        &self,
        group: Uuid,
        part: BridgeGradePart,
        name: String,
    ) -> Result<(), BridgeError> {
        use lumit_core::fx::schema::FxCategory;
        let def = lumit_core::fx::builtins::def(&name).ok_or(BridgeError::UnknownEffectName)?;
        let category = def.schema().category;
        if category.is_driver()
            || category == FxCategory::Compositing
            || name == lumit_core::comp_graph::NODE_GRAPH
            || name == lumit_core::fx::effects::grade_group::GRADE_GROUP
        {
            return Err(BridgeError::NotAStackEffect);
        }
        let comp = self.composition()?;
        let instance = lumit_core::fx::builtins::instantiate_for_raster(
            &name,
            f64::from(comp.width),
            f64::from(comp.height),
        )
        .ok_or(BridgeError::UnknownEffectName)?;
        grade_edit(&*self.project()?, "Edit effects", |doc| {
            let part = part.into();
            let g = groups::group(doc, group).ok_or(GradeError::UnknownGroup)?;
            let mut list = g.part(part).to_vec();
            list.push(instance);
            Ok(((), groups::set_list(doc, group, part, list)?))
        })
    }
}
