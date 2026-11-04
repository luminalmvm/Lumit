//! The user's own Blender, which opens the 3D model files Lumit has no reader
//! for.

use std::path::PathBuf;

use flutter_rust_bridge::frb;
use lumit_core::model::ProjectItem;
use lumit_scene::blender::{self, State};

use crate::api::{footage::FootageReference, project::ProjectReference, BridgeError};

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
    /// What Blender printed about it.
    Failed {
        reason: String,
    },
    /// This Blender has no importer for the file's type.
    NoImporter,
    /// Stopped by hand.
    Stopped,
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

/// Stop every Blender running and wait for them to go, as the app quits. Sync,
/// so quitting never waits on a reply: with none running it returns at once.
#[frb(sync)]
pub fn stop_blenders() {
    blender::stop_where(&|_| true);
    blender::wait_for_stopped();
}

impl ProjectReference {
    /// Stop Blender on any file no item of this project names any more.
    #[frb(sync)]
    pub fn stop_unused_blenders(&self) {
        self.stop_blenders_where(false);
    }

    /// Stop Blender on the files this project names, or on all the others.
    #[frb(ignore)]
    pub(crate) fn stop_blenders_where(&self, named: bool) {
        let Ok(state) = self.state() else {
            return;
        };
        let Ok(state) = state.read() else {
            return;
        };
        let doc = state.store.snapshot();
        blender::stop_where(&|path| {
            let names = |item: &ProjectItem| match item {
                ProjectItem::Footage(f) => std::path::Path::new(f.media.on_disk()) == path,
                _ => false,
            };
            doc.items.iter().any(names) == named
        });
    }
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
            State::NoImporter => BridgeBlenderState::NoImporter,
            State::Stopped => BridgeBlenderState::Stopped,
        }))
    }

    /// Stop this file's conversion.
    #[frb(sync)]
    pub fn stop_blender(&self) -> Result<(), BridgeError> {
        blender::stop(&self.file()?);
        Ok(())
    }

    /// Convert this file again after it was stopped.
    #[frb(sync)]
    pub fn convert_again(&self) -> Result<(), BridgeError> {
        blender::convert_again(&self.file()?);
        Ok(())
    }

    /// Open this file in Blender's own window. False when no Blender was
    /// found or it wouldn't start.
    pub fn edit_in_blender(&self) -> Result<bool, BridgeError> {
        Ok(blender::edit(&self.file()?))
    }
}
