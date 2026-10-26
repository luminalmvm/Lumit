//! The Notes panel's text, kept in the project.

use flutter_rust_bridge::frb;
use lumit_core::Op;

use crate::api::{project::ProjectReference, BridgeError};

impl ProjectReference {
    /// The project's notes, empty when it has none.
    #[frb(sync)]
    pub fn notes(&self) -> Result<String, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state.store.snapshot().notes.clone())
    }

    /// Replace the project's notes. An ordinary op, so it is undoable, shared
    /// and saved in the `.lum`. The same text again is no edit.
    #[frb(sync)]
    pub fn set_notes(&self, notes: String) -> Result<(), BridgeError> {
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        if state.store.snapshot().notes == notes {
            return Ok(());
        }
        state
            .store
            .commit(Op::SetNotes { notes })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }
}
