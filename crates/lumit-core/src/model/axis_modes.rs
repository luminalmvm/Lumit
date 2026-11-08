//! Reading and writing how each two-axis transform pair is edited.

use super::{AxisMode, AxisModes, TransformPair};

impl AxisModes {
    /// Nothing to write: every pair is where a fresh layer leaves it.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn get(&self, pair: TransformPair) -> AxisMode {
        match pair {
            TransformPair::Anchor => self.anchor,
            TransformPair::Position => self.position,
            TransformPair::Scale => self.scale,
        }
    }

    pub fn set(&mut self, pair: TransformPair, mode: AxisMode) {
        match pair {
            TransformPair::Anchor => self.anchor = mode,
            TransformPair::Position => self.position = mode,
            TransformPair::Scale => self.scale = mode,
        }
    }
}
