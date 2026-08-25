use uuid::Uuid;

use flutter_rust_bridge::frb;

use crate::api::{state::PROJECTS, BridgeError};

#[derive(Debug, PartialEq, Eq)]
#[frb]
pub struct SolidReference {
    #[frb(name = "internalproject")]
    pub project: Uuid,
    #[frb(name = "internalid")]
    pub id: Uuid,
}

impl SolidReference {
    #[frb(ignore)]
    pub fn new(project: Uuid, id: Uuid) -> SolidReference {
        SolidReference { project, id }
    }

    #[frb(ignore)]
    pub fn project_id(&self) -> Uuid {
        self.project
    }

    #[frb(ignore)]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The solid asset this reference names, cloned out of the current snapshot.
    #[frb(ignore)]
    pub(crate) fn definition(&self) -> Result<lumit_core::model::SolidDef, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects
            .get(&self.project)
            .ok_or(BridgeError::InvalidProject)?
            .clone();
        drop(projects);

        let state = project.read().map_err(|_| BridgeError::ReadFailed)?;
        match state.store.snapshot().item(self.id) {
            Some(lumit_core::model::ProjectItem::Solid(solid)) => Ok(solid.clone()),
            // A reference that outlived its item, or one whose id was reused by
            // a different kind of item.
            _ => Err(BridgeError::InvalidItem),
        }
    }

    #[frb(ignore)]
    pub(crate) fn commit(&self, op: lumit_core::Op) -> Result<(), BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects
            .get(&self.project)
            .ok_or(BridgeError::InvalidProject)?
            .clone();
        drop(projects);

        let state = project.write().map_err(|_| BridgeError::WriteFailed)?;
        state.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }
}
