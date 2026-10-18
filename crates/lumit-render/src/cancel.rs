//! Stopping a Viewer frame nobody wants any more.
//!
//! The worker sets a token around a render. The build walk checks it between
//! layers and the renderer gives up before compositing, so a newer request
//! waits a layer rather than a whole frame. The composite itself is never cut
//! short: it records into one command buffer and caches precomps as it goes,
//! and stopping half way would leave a cached picture that was never drawn.

use lumit_eval::epoch::EpochToken;
use std::cell::RefCell;

/// The error a cancelled frame returns, so the worker can tell it from a fault.
pub const CANCELLED: &str = "cancelled";

thread_local! {
    static TOKEN: RefCell<Option<EpochToken>> = const { RefCell::new(None) };
}

/// Keeps `token` as this thread's render token until dropped, then puts back
/// whatever was there before.
pub struct Scope(Option<EpochToken>);

impl Scope {
    #[must_use]
    pub fn new(token: EpochToken) -> Self {
        Self(TOKEN.with(|slot| slot.replace(Some(token))))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.0.take();
        TOKEN.with(|slot| *slot.borrow_mut() = previous);
    }
}

/// True when the render on this thread has been asked to stop.
#[must_use]
pub fn requested() -> bool {
    TOKEN.with(|slot| slot.borrow().as_ref().is_some_and(EpochToken::cancelled))
}
