//! Composited-comp rendering for the Viewer — gated behind the `render` feature.
//!
//! # In plain terms
//!
//! The Viewer needs the *real* picture — every layer composited, transformed,
//! blended, with its effects — not one raw footage layer. That compositor lives
//! in `lumit-render`, the engine crate the egui Viewer and the exporter drive
//! too (K-178); the bridge reaches it through its headless seam
//! (`lumit_render::headless`). Nothing here depends on a frontend.
//!
//! Two render entry points, and the difference is the point:
//!
//! - [`render_comp_frame`] serves the Viewer. It names each frame by its content
//!   and serves it from [`crate::framecache`] when it has been rendered before,
//!   so a re-scrubbed frame never touches the GPU.
//! - [`render_preview_frame`] serves a live drag. It re-composites from pixels
//!   the renderer already holds, so a drag tick decodes nothing.
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
    Ready(Box<lumit_render::headless::HeadlessRenderer>),
}

/// The renderer lives behind its OWN lock, distinct from the document lock, so a
/// long render does not block document edits (and vice versa). One Flutter
/// window means one renderer; the lock serialises the render calls it makes.
static RENDERER: OnceLock<Mutex<Slot>> = OnceLock::new();
/// The preview quality one Viewer render asks for. The Dart side sends a single
/// `scale`, which the realtime controller (K-171) drives; below 1.0 it means
/// "this frame is being shown smaller than the comp, so decode it smaller".
///
/// Auto rather than a fixed divisor because the scale is continuous — it tracks
/// the viewport and the adaptive tier, not a Full/Half/Quarter picker.
///
/// Shared with the frb render worker (`api::worker_thread`) rather than copied:
/// two implementations of the same quality policy would drift, and then the two
/// frontends would decode at different sizes for the same on-screen scale.
pub(crate) fn quality_for(scale: f32) -> lumit_render::Quality {
    let sane = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    lumit_render::Quality {
        draft: false,
        auto_res: sane < 1.0,
        display_scale: sane,
        divisor: 1,
    }
}
/// Run `f` against the session-lifetime headless renderer, building it lazily on
/// first use. `None` when the machine has no GPU adapter (the renderer resolves
/// to `Failed` and stays there — a calm, permanent "no frame"). The renderer's
/// own lock serialises the call, separate from the document lock, so a slow
/// render or export prep never blocks an edit. Shared by the Viewer render path
/// and the export-input builder ([`with_export_inputs`]) so both drive the one
/// renderer and share its probe cache.
fn with_ready<R>(f: impl FnOnce(&mut lumit_render::headless::HeadlessRenderer) -> R) -> Option<R> {
    let mutex = RENDERER.get_or_init(|| Mutex::new(Slot::Uninit));
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    if matches!(*guard, Slot::Uninit) {
        *guard = match lumit_render::headless::HeadlessRenderer::new() {
            Ok(renderer) => Slot::Ready(Box::new(renderer)),
            Err(_) => Slot::Failed,
        };
    }
    let Slot::Ready(renderer) = &mut *guard else {
        return None;
    };
    Some(f(renderer))
}
/// Build the footage/audio inputs and a GPU export context for `comp` through
/// the headless seam (K-175), so the export driver can hand them to the exact
/// egui exporter (`lumit_render::export::start`). `None` when the machine has no GPU
/// adapter or the comp is unknown. Reuses the same renderer instance the Viewer
/// path uses, so probes are shared and warm.
pub(crate) fn with_export_inputs(
    doc: &lumit_core::model::Document,
    comp: Uuid,
) -> Option<lumit_render::headless::ExportInputs> {
    with_ready(|renderer| renderer.export_inputs(doc, comp)).flatten()
}
