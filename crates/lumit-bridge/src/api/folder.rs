use flutter_rust_bridge::frb;
use lumit_core::model::ProjectItem;
use uuid::Uuid;

use crate::api::{
    project_item::{item_reference, ItemReference},
    state::PROJECTS,
    BridgeError,
};

#[derive(Debug, PartialEq, Eq)]
#[frb]
pub struct FolderReference {
    #[frb(name = "internalproject")]
    pub project: Uuid,
    #[frb(name = "internalid")]
    pub id: Uuid,
}

impl FolderReference {
    #[frb(ignore)]
    pub fn new(project: Uuid, id: Uuid) -> FolderReference {
        FolderReference { project, id }
    }

    #[frb(ignore)]
    pub fn project_id(&self) -> Uuid {
        self.project
    }

    #[frb(ignore)]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The items this folder lists, in order — one level only, so the Project
    /// panel recurses and draws exactly the rows it needs rather than the engine
    /// flattening a whole tree it may not show.
    ///
    /// A child id that no longer names an item is skipped rather than erroring: a
    /// folder listing a deleted item is a stale document, not a reason to refuse
    /// to draw the panel.
    #[frb(sync)]
    pub fn get_children(&self) -> Result<Vec<ItemReference>, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects
            .get(&self.project)
            .ok_or(BridgeError::InvalidProject)?;
        let p = project.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = p.store.snapshot();

        let folder = match doc.item(self.id).ok_or(BridgeError::InvalidItem)? {
            ProjectItem::Folder(folder) => folder,
            // A FolderReference naming something that is not a folder means the
            // id was reused or the reference outlived its item.
            _ => return Err(BridgeError::InvalidItem),
        };

        Ok(folder
            .children
            .iter()
            .filter_map(|child| doc.item(*child))
            .map(|item| item_reference(self.project, item))
            .collect())
    }
}
