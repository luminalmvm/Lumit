//! One graphics device and one set of compiled shader engines, shared by
//! every test in a process (docs/GUIDE.md).
//!
//! # In plain terms
//!
//! Opening the card takes under a second; compiling the effect engine's
//! hundred-odd kernels takes several more. Two hundred tests that each did
//! both were spending their whole runtime on setup — and had to run one at a
//! time, because a software rasteriser given a dozen devices at once falls
//! over. So the first test to ask builds one of everything, and every later
//! test borrows that one set, one test at a time, with whatever the engines
//! remember (flare bakes, compiled custom shaders, an open frame) wiped
//! between borrowers so no test sees another's leftovers.
//!
//! The borrow is a lock held for the test's duration, which is also what
//! keeps GPU tests serial while the CPU-only tests around them run in
//! parallel. A test that needs a device of its own — the device-loss drill, a
//! second renderer beside the first — still opens one with
//! [`GpuContext::headless`]; asking for the shared one twice on one thread
//! panics rather than deadlocking, and says which fixture to use instead.
//!
//! Only test code reaches this module: `cfg(test)` here, or the
//! `test-fixtures` feature a downstream crate's dev-dependency turns on.
//! Nothing shipped goes through it, which is why the panics below are allowed.

#![allow(clippy::expect_used, clippy::panic)]

use crate::fx::FxEngine;
use crate::scope::ScopeEngine;
use crate::{ColourEngine, Compositor, GpuContext};
use std::ops::Deref;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

/// The device and every engine compiled on it, built once per process.
pub struct SharedGpu {
    pub ctx: GpuContext,
    pub colour: ColourEngine,
    pub compositor: Compositor,
    pub fx: FxEngine,
    pub scope: ScopeEngine,
}

impl SharedGpu {
    fn build() -> Option<Self> {
        let ctx = GpuContext::headless().ok()?;
        Some(Self {
            colour: ColourEngine::new(&ctx),
            compositor: Compositor::new(&ctx),
            fx: FxEngine::new(&ctx),
            scope: ScopeEngine::new(&ctx),
            ctx,
        })
    }

    /// Forget what the previous borrower left behind.
    fn reset(&self) {
        // A test that panicked mid-frame leaves the encoder open; the next
        // test starts on a closed one.
        self.ctx.frame.replace(None);
        self.ctx.frame_depth.set(0);
        self.fx.reset_for_tests();
    }
}

static POOL: Mutex<Option<SharedGpu>> = Mutex::new(None);
/// Which thread holds the pool right now, so a re-entrant ask is a panic with
/// a message rather than a silent deadlock.
static HOLDER: Mutex<Option<ThreadId>> = Mutex::new(None);

/// The shared device and engines, borrowed for the rest of the test. Derefs
/// to the [`GpuContext`], so `&lease` goes wherever `&GpuContext` went; the
/// engines are [`Lease::fx`], [`Lease::colour`], [`Lease::compositor`] and
/// [`Lease::scope`].
pub struct Lease {
    guard: MutexGuard<'static, Option<SharedGpu>>,
}

/// Borrow the process's shared GPU, building it on the first call. `None`
/// when the machine has no adapter — call [`crate::no_adapter`] and return,
/// exactly as with a device of one's own.
pub fn lease() -> Option<Lease> {
    let me = std::thread::current().id();
    let held_here = HOLDER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some_and(|holder| holder == me);
    assert!(
        !held_here,
        "the shared GPU fixture is already leased on this thread: a test that needs a second device or renderer opens its own (GpuContext::headless, HeadlessRenderer::new)"
    );
    let mut guard = POOL.lock().unwrap_or_else(PoisonError::into_inner);
    // A test that lost the device for real leaves the next one a fresh card.
    if guard.as_ref().is_some_and(|s| s.ctx.device_lost()) {
        *guard = None;
    }
    if guard.is_none() {
        *guard = Some(SharedGpu::build()?);
    }
    guard.as_ref()?.reset();
    *HOLDER.lock().unwrap_or_else(PoisonError::into_inner) = Some(me);
    Some(Lease { guard })
}

impl Lease {
    fn shared(&self) -> &SharedGpu {
        self.guard
            .as_ref()
            .expect("the shared GPU was taken out of its lease and not restored")
    }

    #[must_use]
    pub fn fx(&self) -> &FxEngine {
        &self.shared().fx
    }

    #[must_use]
    pub fn colour(&self) -> &ColourEngine {
        &self.shared().colour
    }

    #[must_use]
    pub fn compositor(&self) -> &Compositor {
        &self.shared().compositor
    }

    #[must_use]
    pub fn scope(&self) -> &ScopeEngine {
        &self.shared().scope
    }

    /// Take the device and engines out, for a fixture that has to *own*
    /// them for a while (lumit-render's headless renderer does). Give them
    /// back with [`Lease::restore`] before the lease drops; a set that is
    /// never returned is rebuilt by the next borrower, which is slow but
    /// correct.
    pub fn take(&mut self) -> Option<SharedGpu> {
        self.guard.take()
    }

    pub fn restore(&mut self, shared: SharedGpu) {
        *self.guard = Some(shared);
    }
}

impl Deref for Lease {
    type Target = GpuContext;

    fn deref(&self) -> &GpuContext {
        &self.shared().ctx
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        *HOLDER.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}
