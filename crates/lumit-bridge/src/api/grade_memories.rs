//! The project's grade memories: eight numbered slots, each a kept grade
//! ([`lumit_core::grade::memories`]).

use flutter_rust_bridge::frb;

use lumit_core::grade::memories::{self, GradeMemory};

use crate::api::grade_versions::grade_edit;
use crate::api::project::ProjectReference;
use crate::api::BridgeError;

/// One filled slot.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGradeMemory {
    /// 1 to 8.
    pub slot: u8,
    /// The grade, as the effects text Copy grade writes.
    pub grade: String,
    /// A small PNG of the shot it came from, base64, or empty.
    pub picture: String,
}

impl ProjectReference {
    /// The filled slots, in slot order.
    #[frb(sync)]
    pub fn get_grade_memories(&self) -> Result<Vec<BridgeGradeMemory>, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state
            .store
            .snapshot()
            .grade_memories
            .iter()
            .map(|m| BridgeGradeMemory {
                slot: m.slot,
                grade: m.grade.clone(),
                picture: m.picture.clone(),
            })
            .collect())
    }

    /// Keep `grade` in `slot`, with its picture, or with `None` empty the
    /// slot. One undo step.
    #[frb(sync)]
    pub fn set_grade_memory(
        &self,
        slot: u8,
        grade: Option<String>,
        picture: String,
    ) -> Result<(), BridgeError> {
        grade_edit(&*self.state()?, "Store memory", |doc| {
            let memory = grade.map(|grade| GradeMemory {
                slot,
                grade,
                picture,
            });
            Ok(((), memories::set(doc, slot, memory)))
        })
    }
}
