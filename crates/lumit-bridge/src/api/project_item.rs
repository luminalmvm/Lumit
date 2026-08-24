use std::sync::{Arc, RwLock};

use flutter_rust_bridge::frb;
use lumit_core::model::ProjectItem;
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference,
    folder::FolderReference,
    footage::FootageReference,
    solid::SolidReference,
    state::{LumitBridgeState, PROJECTS},
    BridgeError,
};

#[frb(non_opaque)]
#[derive(Debug, PartialEq, Eq)]
pub enum ItemReference {
    Footage(FootageReference),
    Solid(SolidReference),
    Composition(CompositionReference),
    Folder(FolderReference),
}

#[frb(name = "ItemInfo")]
#[frb(non_opaque)]
pub struct LumitProjectItemInfo {
    pub name: String,
}

/// The reference naming `item` in `project`.
///
/// One place rather than inline at each site, because both `get_items` (the
/// panel's roots) and `FolderReference::get_children` (everything nested) build
/// these, and a new `ProjectItem` variant must not be able to reach only one of
/// them — the compiler catches the missing arm here, once.
#[frb(ignore)]
pub(crate) fn item_reference(project: Uuid, item: &ProjectItem) -> ItemReference {
    match item {
        ProjectItem::Composition(_) => {
            ItemReference::Composition(CompositionReference::new(project, item.id()))
        }
        ProjectItem::Folder(_) => ItemReference::Folder(FolderReference::new(project, item.id())),
        ProjectItem::Solid(_) => ItemReference::Solid(SolidReference::new(project, item.id())),
        ProjectItem::Footage(_) => {
            ItemReference::Footage(FootageReference::new(project, item.id()))
        }
    }
}

impl ItemReference {
    #[frb(sync)]
    pub fn equals(&self, item: &ItemReference) -> bool {
        self == item
    }

    /// The project this item belongs to.
    #[frb(ignore)]
    pub(crate) fn project_id(&self) -> Uuid {
        match self {
            ItemReference::Footage(r) => r.project_id(),
            ItemReference::Solid(r) => r.project_id(),
            ItemReference::Composition(r) => r.project_id(),
            ItemReference::Folder(r) => r.project_id(),
        }
    }

    /// The item's own id, whatever kind it is.
    #[frb(ignore)]
    pub(crate) fn item_id(&self) -> Uuid {
        match self {
            ItemReference::Footage(r) => r.id(),
            ItemReference::Solid(r) => r.id(),
            ItemReference::Composition(r) => r.id(),
            ItemReference::Folder(r) => r.id(),
        }
    }

    /// Commit `op` against this item's project.
    #[frb(ignore)]
    fn commit(&self, op: lumit_core::Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Rename the item — the panel's in-place rename.
    ///
    /// A blank name is refused rather than applied, matching v0: the field keeps
    /// the old name instead of the row losing its label.
    #[frb(sync)]
    pub fn rename(&self, name: String) -> Result<(), BridgeError> {
        if name.trim().is_empty() {
            return Err(BridgeError::EmptyName);
        }
        // Confirm the item exists before committing, so an unknown id is a calm
        // error rather than a failed op.
        self.item()?;
        self.commit(lumit_core::Op::RenameItem {
            id: self.item_id(),
            name,
        })
    }

    /// Delete the item. One undo step, no confirmation — matching the egui
    /// project menu, where Delete is undoable and therefore not worth a dialog.
    #[frb(sync)]
    pub fn delete(&self) -> Result<(), BridgeError> {
        self.item()?;
        self.commit(lumit_core::Op::RemoveItem { id: self.item_id() })
    }

    /// Move the item back to the panel root: remove it from every folder that
    /// lists it, as one undo step. Already at the root is a calm no-op.
    #[frb(sync)]
    pub fn move_to_root(&self) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let id = self.item_id();

        let ops = {
            let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = p.store.snapshot();
            if doc.item(id).is_none() {
                return Err(BridgeError::InvalidItem);
            }
            doc.items
                .iter()
                .filter_map(|pi| match pi {
                    ProjectItem::Folder(f) if f.children.contains(&id) => {
                        Some(lumit_core::Op::SetFolderChildren {
                            folder: f.id,
                            children: f.children.iter().copied().filter(|c| *c != id).collect(),
                        })
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        match ops.len() {
            // Nothing lists it — it is already at the root.
            0 => Ok(()),
            1 => self.commit(ops.into_iter().next().ok_or(BridgeError::InvalidItem)?),
            // One undo step for the whole move, however many folders listed it.
            _ => self.commit(lumit_core::Op::Batch { ops }),
        }
    }

    /// File the item into `folder`: out of whatever folder listed it, onto the
    /// end of that one, as one undo step. The panel's drag onto a folder row
    /// and its **Move to folder** menu entry both land here.
    ///
    /// The composition is the engine's ([`lumit_core::ops::move_to_folder_ops`]),
    /// and so are the refusals: an item or folder that no longer exists is
    /// [`BridgeError::InvalidItem`], and a folder asked to move inside itself or
    /// its own descendant is [`BridgeError::FolderCycle`] — that move would take
    /// the branch off the panel root with nothing left to drag it back by.
    /// Filing something where it already sits is a calm no-op rather than an
    /// undo step that changed nothing.
    #[frb(sync)]
    pub fn move_to_folder(&self, folder: Uuid) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let id = self.item_id();

        let ops = {
            let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = p.store.snapshot();
            if doc.item(id).is_none() || doc.folder(folder).is_none() {
                return Err(BridgeError::InvalidItem);
            }
            lumit_core::ops::move_to_folder_ops(&doc, id, folder).ok_or(BridgeError::FolderCycle)?
        };

        match ops.len() {
            // It is already there.
            0 => Ok(()),
            1 => self.commit(ops.into_iter().next().ok_or(BridgeError::InvalidItem)?),
            // One undo step for the whole move, however many folders it touched.
            _ => self.commit(lumit_core::Op::Batch { ops }),
        }
    }

    fn project(&self) -> Result<Arc<RwLock<LumitBridgeState>>, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects
            .get(&self.project_id())
            .ok_or(BridgeError::InvalidProject)?;

        Ok(project.clone())
    }

    fn item(&self) -> Result<ProjectItem, BridgeError> {
        let proj = self.project()?;
        let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = p.store.snapshot();
        let item = snapshot
            .item(self.item_id())
            .ok_or(BridgeError::InvalidItem)?;
        Ok(item.clone())
    }

    #[frb(sync)]
    pub fn name(&self) -> Result<String, BridgeError> {
        let item = self.item()?;

        Ok(item.name().to_string())
    }

    /// Whether any composition places this item as a layer — the panel's
    /// `in use` badge (docs/07 §3.1, docs/15 §12A.3a).
    ///
    /// Direct placement only, deliberately, and that rule is the engine's
    /// ([`lumit_core::Document::item_is_used`]): the badge says "a layer
    /// somewhere names this", not "some render might reach it", so it does not
    /// come and go as an unrelated comp is nested elsewhere.
    #[frb(sync)]
    pub fn is_used(&self) -> Result<bool, BridgeError> {
        let proj = self.project()?;
        let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = p.store.snapshot();
        let id = self.item_id();
        if doc.item(id).is_none() {
            return Err(BridgeError::InvalidItem);
        }
        Ok(doc.item_is_used(id))
    }

    /// This item's colour tag: an index into the same label palette a layer's
    /// chip uses, `0` for untagged (K-451). Every item of a project saved
    /// before tags existed answers 0.
    #[frb(sync)]
    pub fn label(&self) -> Result<u8, BridgeError> {
        let proj = self.project()?;
        let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = p.store.snapshot();
        let id = self.item_id();
        if doc.item(id).is_none() {
            return Err(BridgeError::InvalidItem);
        }
        Ok(doc.item_label(id))
    }

    /// Tag this item, or untag it with `0`. One undo step.
    ///
    /// Untagging leaves the document exactly as it was found — the engine
    /// stores tags as a map beside the items and removes the entry rather than
    /// writing a zero — so a project nobody has tagged gains no line in the
    /// file (K-258).
    #[frb(sync)]
    pub fn set_label(&self, label: u8) -> Result<(), BridgeError> {
        self.item()?;
        self.commit(lumit_core::Op::SetItemLabel {
            id: self.item_id(),
            label,
        })
    }
}
