//! Versions of a shot's grade: named alternatives kept side by side and
//! switched between, each switch one undo step ([`lumit_core::grade::versions`]).
//!
//! A shot is a layer, or with `clip` one clip on a Sequence layer.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use lumit_core::grade::{GradeError, Shot};
use lumit_core::model::Document;
use lumit_core::ops::Op;
use lumit_core::store::DocumentStore;

use crate::api::layer::LayerReference;
use crate::api::BridgeError;

/// One version of a shot's grade.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGradeVersion {
    pub id: Uuid,
    pub name: String,
    /// Whether it is the one the shot shows.
    pub live: bool,
}

/// A grade edit's refusal as the bridge says it.
#[frb(ignore)]
pub(crate) fn refused(e: GradeError) -> BridgeError {
    match e {
        GradeError::UnknownShot => BridgeError::InvalidLayer,
        GradeError::UnknownVersion | GradeError::UnknownGroup => BridgeError::InvalidItem,
        GradeError::UnknownNode => BridgeError::InvalidEffect,
        GradeError::EmptyName => BridgeError::EmptyName,
        GradeError::Op(e) => BridgeError::OpError(e),
    }
}

/// Whether an effect called `name` is part of a grade: what a version keeps
/// and Copy grade carries. A group's reference is not.
#[frb(sync)]
pub fn is_grade_effect(name: String) -> bool {
    lumit_core::grade::is_grade_name(&name)
}

/// Commit `op` as one undo step called `name` in the History list. A caller
/// that already holds a group open keeps its own name for it.
#[frb(ignore)]
pub(crate) fn commit_named(
    store: &DocumentStore,
    name: &'static str,
    op: Option<Op>,
) -> Result<(), BridgeError> {
    let Some(op) = op else {
        return Ok(());
    };
    let own = store.begin_own_undo_group();
    if own {
        store.name_undo_group(name);
    }
    let done = store.commit(op);
    if own {
        store.end_undo_group();
    }
    done.map(|_| ()).map_err(BridgeError::OpError)
}

/// Work out an edit against the document as it stands and commit it, under
/// the project's write lock so nothing lands in between.
#[frb(ignore)]
pub(crate) fn grade_edit<T>(
    state: &std::sync::RwLock<crate::api::state::LumitBridgeState>,
    name: &'static str,
    edit: impl FnOnce(&Document) -> Result<(T, Option<Op>), GradeError>,
) -> Result<T, BridgeError> {
    let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
    let doc = state.store.snapshot();
    let (answer, op) = edit(&doc).map_err(refused)?;
    commit_named(&state.store, name, op)?;
    Ok(answer)
}

impl LayerReference {
    #[frb(ignore)]
    pub(crate) fn shot(&self, clip: Option<Uuid>) -> Shot {
        Shot {
            comp: self.comp_id(),
            layer: self.id(),
            clip,
        }
    }

    /// The shot's versions in the order they were made. Empty for a shot
    /// that has none.
    #[frb(sync)]
    pub fn get_grade_versions(
        &self,
        clip: Option<Uuid>,
    ) -> Result<Vec<BridgeGradeVersion>, BridgeError> {
        let doc = self.document()?;
        let (_, versions) = lumit_core::grade::read(&doc, self.shot(clip))
            .map_err(crate::api::grade_versions::refused)?;
        Ok(versions.map_or_else(Vec::new, |set| {
            set.versions
                .iter()
                .map(|v| BridgeGradeVersion {
                    id: v.id,
                    name: v.name.clone(),
                    live: v.id == set.live,
                })
                .collect()
        }))
    }

    /// Add a version and show it: a copy of the grade showing, or with `copy`
    /// off an empty one. Answers its id.
    #[frb(sync)]
    pub fn add_grade_version(&self, clip: Option<Uuid>, copy: bool) -> Result<Uuid, BridgeError> {
        let shot = self.shot(clip);
        grade_edit(&*self.project()?, "New grade version", |doc| {
            lumit_core::grade::versions::add(doc, shot, copy)
        })
    }

    /// Show version `version` in place of the one showing.
    #[frb(sync)]
    pub fn switch_grade_version(
        &self,
        clip: Option<Uuid>,
        version: Uuid,
    ) -> Result<(), BridgeError> {
        let shot = self.shot(clip);
        grade_edit(&*self.project()?, "Switch grade version", |doc| {
            Ok(((), lumit_core::grade::versions::switch(doc, shot, version)?))
        })
    }

    #[frb(sync)]
    pub fn rename_grade_version(
        &self,
        clip: Option<Uuid>,
        version: Uuid,
        name: String,
    ) -> Result<(), BridgeError> {
        let shot = self.shot(clip);
        grade_edit(&*self.project()?, "Rename grade version", |doc| {
            Ok((
                (),
                lumit_core::grade::versions::rename(doc, shot, version, &name)?,
            ))
        })
    }

    /// Delete a version. The one showing goes only when it is the last, and
    /// its grade stays on the shot.
    #[frb(sync)]
    pub fn delete_grade_version(
        &self,
        clip: Option<Uuid>,
        version: Uuid,
    ) -> Result<(), BridgeError> {
        let shot = self.shot(clip);
        grade_edit(&*self.project()?, "Delete grade version", |doc| {
            Ok(((), lumit_core::grade::versions::delete(doc, shot, version)?))
        })
    }
}
