//! Clearing the History window's list, and naming a step in it.

use flutter_rust_bridge::frb;

use crate::api::{project::ProjectReference, BridgeError};

impl ProjectReference {
    /// Drop every undo and redo step and leave the document as it is. Under
    /// the project's lock, as undo and redo are.
    #[frb(sync)]
    pub fn clear_history(&self) -> Result<(), BridgeError> {
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state.store.clear_history();
        Ok(())
    }

    /// Call the step the open undo group folds into `name` in the History
    /// list, rather than after the first thing done in it. English, translated
    /// on arrival like every other step's name. Does nothing with no group open.
    #[frb(sync)]
    pub fn name_undo_group(&self, name: String) -> Result<(), BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.name_undo_group(name);
        Ok(())
    }
}
