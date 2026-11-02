use super::HeadlessRenderer;
use lumit_gpu::test_support::{lease, Lease};
use lumit_gpu::SharedGpu;
use std::ops::{Deref, DerefMut};

/// A [`HeadlessRenderer`] on borrowed engines. Derefs to the renderer;
/// returns the engines on drop.
pub struct TestRenderer {
    /// `Some` for the whole of the test; taken in `drop`.
    renderer: Option<HeadlessRenderer>,
    lease: Lease,
}

impl HeadlessRenderer {
    /// A renderer around the shared device and engines, or `Err` on a
    /// machine with no adapter — the same shape as [`Self::new`], so a
    /// test's skip site does not change.
    pub fn shared() -> Result<TestRenderer, String> {
        let mut lease = lease().ok_or_else(|| "no GPU adapter".to_string())?;
        let shared = lease
            .take()
            .ok_or_else(|| "the shared GPU is out on loan".to_string())?;
        let SharedGpu {
            ctx,
            colour,
            compositor,
            fx,
            scope,
        } = shared;
        let renderer = HeadlessRenderer::from_engines(ctx, colour, compositor, fx, scope);
        Ok(TestRenderer {
            renderer: Some(renderer),
            lease,
        })
    }
}

impl Deref for TestRenderer {
    type Target = HeadlessRenderer;

    // Test-only code; the `Option` is `Some` until `drop` by construction.
    #[allow(clippy::expect_used)]
    fn deref(&self) -> &HeadlessRenderer {
        self.renderer
            .as_ref()
            .expect("a test renderer outlives its drop")
    }
}

impl DerefMut for TestRenderer {
    #[allow(clippy::expect_used)]
    fn deref_mut(&mut self) -> &mut HeadlessRenderer {
        self.renderer
            .as_mut()
            .expect("a test renderer outlives its drop")
    }
}

impl Drop for TestRenderer {
    fn drop(&mut self) {
        if let Some(shared) = self
            .renderer
            .take()
            .and_then(HeadlessRenderer::into_engines)
        {
            self.lease.restore(shared);
        }
    }
}
