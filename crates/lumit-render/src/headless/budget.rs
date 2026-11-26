//! What the renderer may hold, and what it is holding.

use super::HeadlessRenderer;

impl HeadlessRenderer {
    /// What the resource governor is holding, across both tiers
    /// (docs/13 §3) — every store on this renderer that has registered with it,
    /// plus whatever frame is in flight.
    ///
    /// The readout the degradation ladder owes the user: "silent degradation is
    /// a bug" (docs/13 §4), and a ladder nobody can see stepping is exactly
    /// that. Read in one go so the numbers shown together were true together.
    #[must_use]
    pub fn governor(&self) -> lumit_budget::Snapshot {
        self.gpu.ledger().snapshot()
    }

    /// Set the governor's two ceilings (docs/13 §3), from the figures the
    /// frontend read off the machine.
    ///
    /// The renderer sized the card's tier from its own adapter as it opened,
    /// which is all a graphics context can honestly answer for. This is where
    /// the rest arrives: what the machine has in ordinary memory, and — on a
    /// card that draws from that same memory — the share of it the card may
    /// have, which needed both figures to work out and so could not be decided
    /// either side alone.
    ///
    /// Changing a ceiling frees nothing and takes nothing away: the
    /// reservations that exist are memory that exists. What it changes is what
    /// is granted from here on, and where the degradation ladder starts
    /// stepping.
    pub fn set_memory_budgets(&self, vram: u64, ram: u64) {
        self.gpu.ledger().set_budget(lumit_budget::Tier::Vram, vram);
        self.gpu.ledger().set_budget(lumit_budget::Tier::Ram, ram);
    }

    /// Bytes the last frame drawn asked the card for and did not get — see
    /// [`lumit_gpu::GpuContext::last_frame_overdrawn`]. Nought on every
    /// ordinary frame; anything else is a frame that outgrew what was reserved
    /// for it. The *latched* figure, because this is read between frames,
    /// where the live one has always just been reset.
    #[must_use]
    pub fn vram_overdrawn(&self) -> u64 {
        self.gpu.last_frame_overdrawn()
    }

    /// Skip, rather than draw, a frame that can never fit the budget. See
    /// [`lumit_gpu::GpuContext::refuse_frames_over_budget`].
    pub fn refuse_frames_over_budget(&self, on: bool) {
        self.gpu.refuse_frames_over_budget(on);
    }

    /// Whether those are skipped, which is also whether the video memory
    /// budget is one this machine is held to.
    #[must_use]
    pub fn refuses_frames_over_budget(&self) -> bool {
        self.gpu.refuses_frames_over_budget()
    }

    /// How many frames so far could never fit the budget. Read it either side
    /// of a render to tell whether that one was one of them.
    #[must_use]
    pub fn frames_over_budget(&self) -> u64 {
        self.gpu.frames_over_budget()
    }

    /// The last frame that could never fit the budget.
    #[must_use]
    pub fn over_budget(&self) -> Option<lumit_gpu::frame_budget::OverBudget> {
        self.gpu.over_budget()
    }

    /// The frame just made, if it could never fit and was skipped rather than
    /// drawn. `before` is [`Self::frames_over_budget`] from before it.
    pub(super) fn skipped_over_budget(
        &self,
        before: u64,
    ) -> Option<lumit_gpu::frame_budget::OverBudget> {
        (self.refuses_frames_over_budget() && self.frames_over_budget() != before)
            .then(|| self.over_budget())
            .flatten()
    }
}
