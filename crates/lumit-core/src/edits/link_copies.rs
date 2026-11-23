//! What a layer moved out of its comp leaves behind.

use super::*;

impl LinkCopies {
    pub fn is_empty(&self) -> bool {
        self.inside.is_empty() && self.outside.is_empty()
    }
}
