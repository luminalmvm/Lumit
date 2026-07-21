//! Composited-comp rendering for the Viewer — gated behind the `render` feature.
//!
//! # In plain terms
//!
//! The Viewer needs the *real* picture — every layer composited, transformed,
//! blended, with its effects — not one raw footage layer. That compositor lives
//! in `lumit-ui` (it is the same code the egui Viewer and the exporter use). The
//! bridge borrows it here through the headless seam (`lumit_ui::headless`), a
//! deliberate temporary arrangement recorded as K-175: the bridge reaches into
//! the UI crate's renderer until the pixel pass moves into an engine crate.
//!
//! The GPU renderer is expensive to build (it acquires an adapter and compiles
//! shaders), so it is created **once**, lazily, on the first render call and
//! kept alive for the session behind its own lock — separate from the document
//! lock, so a slow render never blocks an edit. A machine with no GPU adapter
//! resolves to a calm "unavailable" state on that first call and stays there:
//! every render then returns null (never a crash, never a retry storm).
//!
//! Without the `render` feature this module is absent and
//! [`crate::ffi::lumit_bridge_render_comp_frame`] always returns null.

#![cfg(feature = "render")]

use crate::state::with_bridge;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// The session-lifetime renderer, created lazily on first use. `Failed` is the
/// calm terminal state for a machine with no GPU adapter (or a device that would
/// not open): once there, every render returns null without retrying.
enum Slot {
    /// Not yet asked to render — the adapter has not been touched.
    Uninit,
    /// Adapter acquisition or device open failed once; stay here.
    Failed,
    /// A live renderer, holding its GPU context, engines and decoder pool.
    /// Boxed: the renderer is far larger than the empty variants, so the enum
    /// stays small and moving it between states is a pointer move.
    Ready(Box<lumit_ui::headless::HeadlessRenderer>),
}

/// The renderer lives behind its OWN lock, distinct from the document lock, so a
/// long render does not block document edits (and vice versa). One Flutter
/// window means one renderer; the lock serialises the render calls it makes.
static RENDERER: OnceLock<Mutex<Slot>> = OnceLock::new();

/// Render composition `comp_id` at `frame` to tightly-packed RGBA8, returning
/// `(width, height, pixels)`. `None` on any failure — an unknown/invalid comp
/// id, no GPU adapter, or a render error — mirroring `decode_frame`'s null
/// contract so the FFI boundary treats it as "no frame". `scale` of 1.0 is the
/// comp's own resolution; a smaller positive value downsamples the output.
pub(crate) fn render_comp_frame(
    comp_id: &str,
    frame: u64,
    scale: f32,
) -> Option<(u32, u32, Vec<u8>)> {
    let comp = Uuid::parse_str(comp_id).ok()?;
    // Take a cheap snapshot (an `Arc<Document>` clone) under the document lock,
    // then release it before the slow GPU work under the renderer lock.
    let doc = with_bridge(|b| b.store.snapshot());

    let mutex = RENDERER.get_or_init(|| Mutex::new(Slot::Uninit));
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());

    // First render on this session builds (or fails to build) the renderer.
    if matches!(*guard, Slot::Uninit) {
        *guard = match lumit_ui::headless::HeadlessRenderer::new() {
            Ok(renderer) => Slot::Ready(Box::new(renderer)),
            Err(_) => Slot::Failed,
        };
    }

    let Slot::Ready(renderer) = &mut *guard else {
        return None; // Failed (no adapter) — a calm, permanent "no frame".
    };
    renderer
        .render_rgba(&doc, comp, frame, scale)
        .ok()
        .map(|(rgba, w, h)| (w, h, rgba))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// An unparseable comp id is `None` before any GPU work — so this holds even
    /// on a machine with no adapter (CI without a GPU).
    #[test]
    fn a_bad_comp_id_is_none() {
        assert!(render_comp_frame("not-a-uuid", 0, 1.0).is_none());
    }

    /// A well-formed but unknown comp id is `None`. On a GPU-less machine the
    /// renderer resolves to `Failed` and this is `None` via that path; on a GPU
    /// machine it is `None` because the comp does not exist — either way, never a
    /// panic. (A real solid-comp render is exercised in `lumit-ui`'s headless
    /// tests, which own the synthetic document; here the bridge holds an empty
    /// document, so there is no comp to render.)
    #[test]
    fn an_unknown_comp_is_none() {
        let unknown = Uuid::now_v7().to_string();
        assert!(render_comp_frame(&unknown, 0, 1.0).is_none());
    }
}
