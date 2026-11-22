//! The resolved stacks one drivers pass lends out and takes back.

use super::*;

impl ArenaPool {
    #[cfg(test)]
    pub(super) fn checked_out(&mut self, reused: bool) {
        if reused {
            self.reuses += 1;
        } else {
            self.allocations += 1;
        }
    }

    #[cfg(not(test))]
    pub(super) fn checked_out(&mut self, _reused: bool) {}

    #[cfg(test)]
    pub(super) fn returned(&mut self) {
        self.high_water = self.high_water.max(self.arenas.len());
    }

    #[cfg(not(test))]
    pub(super) fn returned(&mut self) {}
}
