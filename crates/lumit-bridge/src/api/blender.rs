//! The user's own Blender, which opens the 3D model files Lumit has no reader
//! for.

use std::path::PathBuf;

use flutter_rust_bridge::frb;
use lumit_core::model::ProjectItem;
use lumit_scene::blender::{self, State};

use crate::api::{footage::FootageReference, BridgeError};

/// The Blender 3D models open with.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeBlender {
    pub path: String,
    /// As it names itself, "5.2.3".
    pub version: String,
}

/// Where a 3D model's trip through Blender stands, while it isn't done.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeBlenderState {
    Converting,
    NoBlender,
    /// The last lines Blender printed.
    Failed {
        reason: String,
    },
}

/// Try the program at `path` before any Blender on the machine, or none when
/// empty. The interface keeps it and gives it again at each start.
#[frb(sync)]
pub fn choose_blender(path: String) {
    blender::choose((!path.is_empty()).then(|| path.into()));
}

/// The Blender 3D models open with, or none. Can take a few seconds the first
/// time, as each one found is asked its version.
pub fn find_blender() -> Option<BridgeBlender> {
    blender::find().map(|found| BridgeBlender {
        path: found.exe.to_string_lossy().into_owned(),
        version: found.version,
    })
}

impl FootageReference {
    /// The file on disk.
    fn file(&self) -> Result<PathBuf, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        match proj.store.snapshot().item(self.id()) {
            Some(ProjectItem::Footage(f)) => Ok(PathBuf::from(f.media.on_disk())),
            _ => Err(BridgeError::ReadFailed),
        }
    }

    /// Where this 3D model's trip through Blender stands, `None` when it
    /// needs none or is done. Cheap: it reads nothing.
    #[frb(sync)]
    pub fn blender_state(&self) -> Result<Option<BridgeBlenderState>, BridgeError> {
        Ok(blender::state(&self.file()?).map(|state| match state {
            State::Converting => BridgeBlenderState::Converting,
            State::NoBlender => BridgeBlenderState::NoBlender,
            State::Failed(reason) => BridgeBlenderState::Failed { reason },
        }))
    }

    /// Open this file in Blender's own window. False when no Blender was
    /// found or it wouldn't start.
    pub fn edit_in_blender(&self) -> Result<bool, BridgeError> {
        Ok(blender::edit(&self.file()?))
    }
}
