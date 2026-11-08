//! Clearing the History window's list.

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
}
