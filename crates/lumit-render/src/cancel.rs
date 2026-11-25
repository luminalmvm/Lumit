//! Stopping a Viewer frame nobody wants any more.
//!
//! The worker sets a token around a render. The build walk checks it between
//! layers and the renderer gives up before compositing, so a newer request
//! waits a layer rather than a whole frame. The composite itself is never cut
//! short: it records into one command buffer and caches precomps as it goes,
//! and stopping half way would leave a cached picture that was never drawn.
//! Particulate's Trails do stop inside it, so once a stop is asked nothing
//! more is filed, and a frame a trail cut short is not shown. A frame that
//! finished is shown even if a newer one is waiting.

use lumit_eval::epoch::EpochToken;
use std::cell::{Cell, RefCell};

pub const CANCELLED: &str = "cancelled";

thread_local! {
    static TOKEN: RefCell<Option<EpochToken>> = const { RefCell::new(None) };
    static SHORT: Cell<bool> = const { Cell::new(false) };
}

pub fn cut_short() {
    SHORT.with(|s| s.set(true));
}

#[must_use]
pub fn drawn_short() -> bool {
    SHORT.with(|s| s.replace(false))
}

/// Keeps `token` as this thread's render token until dropped, then puts back
/// whatever was there before. Particulate's walk cannot see the token, so it
/// is handed [`requested`] to ask with.
pub struct Scope(Option<EpochToken>, Option<fn() -> bool>);

impl Scope {
    #[must_use]
    pub fn new(token: EpochToken) -> Self {
        let walk = lumit_core::fx::points::stepped::stop_when(Some(requested));
        Self(TOKEN.with(|slot| slot.replace(Some(token))), walk)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.0.take();
        TOKEN.with(|slot| *slot.borrow_mut() = previous);
        lumit_core::fx::points::stepped::stop_when(self.1);
    }
}

#[must_use]
pub fn or_fault(error: String, what: &str) -> String {
    if error == CANCELLED {
        error
    } else {
        format!("{what}: {error}")
    }
}

#[must_use]
pub fn requested() -> bool {
    TOKEN.with(|slot| slot.borrow().as_ref().is_some_and(EpochToken::cancelled))
}
