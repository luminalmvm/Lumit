//! Telling a frame the card drew from one it could not.

use super::HeadlessRenderer;
use lumit_gpu::device::Fault;

impl HeadlessRenderer {
    /// Watch what the card refuses, until [`Self::faults`]. See
    /// [`lumit_gpu::GpuContext::watch_faults`].
    pub fn watch_faults(&self) {
        self.gpu.watch_faults();
    }

    /// The first thing that went wrong since [`Self::watch_faults`], if
    /// anything did. A lost device is struck against what was running, as
    /// the render worker does.
    #[must_use]
    pub fn faults(&self) -> Option<Fault> {
        let fault = self.gpu.faults();
        if fault.is_some() {
            let _ = self.device_lost();
        }
        fault
    }
}
