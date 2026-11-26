//! Keeping the card from falling too far behind what has been handed to it.

/// Submissions handed to one device and not yet waited for, oldest first.
/// Shared by every handle on the device, because it is the one card that
/// falls behind.
pub(crate) type Behind =
    std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<wgpu::SubmissionIndex>>>;

/// How many submissions a device is left with before [`crate::GpuContext::submit`]
/// waits for the oldest.
///
/// The driver keeps the command buffers of every submission until the card has
/// run it, and keeps them for reuse afterwards in a pool that never shrinks. A
/// frame with Echo over motion-blurred layers made hundreds of submissions
/// faster than the card ran them, and the process grew by gigabytes outside any
/// count the ledger keeps. A card that keeps up never waits here.
///
/// Four was measured on that project: it held the process flat and exported
/// faster than no limit at all. Two was as slow as no limit, and eight and
/// more let it grow again.
pub(crate) const SUBMITS_IN_FLIGHT: usize = 4;

impl crate::GpuContext {
    /// Note `index` in `behind`, and wait for the oldest once more than `most`
    /// are there.
    pub(crate) fn keep_within(&self, behind: &Behind, index: wgpu::SubmissionIndex, most: usize) {
        // The lock is let go before the wait: another handle meanwhile waits
        // for its own oldest, not for this one.
        let oldest = {
            let Ok(mut queue) = behind.lock() else {
                return;
            };
            queue.push_back(index);
            if queue.len() <= most {
                return;
            }
            queue.pop_front()
        };
        // A lost device answers no wait.
        if let Some(oldest) = oldest.filter(|_| !self.device_lost()) {
            let done = self
                .device
                .poll(wgpu::Maintain::WaitForSubmissionIndex(oldest));
            self.forget_finished(done);
        }
    }
}
