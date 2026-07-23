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
///
/// The result is served from the bridge-side rendered-frame cache
/// ([`crate::framecache`]) when this comp/frame/scale was already rendered under
/// the current document — a re-scrubbed frame skips the GPU entirely. The legacy
/// (generation-less) entry point; [`render_comp_frame_gen`] adds latest-wins
/// cancellation for the worker's newer calls.
pub(crate) fn render_comp_frame(
    comp_id: &str,
    frame: u64,
    scale: f32,
) -> Option<(u32, u32, Vec<u8>)> {
    render_comp_frame_gen(comp_id, frame, scale, 0)
}

/// [`render_comp_frame`] with a `generation` for latest-wins cancellation
/// (K-176): a cache hit is served regardless of generation, but a cache **miss**
/// aborts before the GPU render when a newer request has already superseded this
/// one (see [`crate::cancel`]). So a stale render queued behind the renderer lock
/// is skipped rather than stealing a full render from the frame the user wants.
pub(crate) fn render_comp_frame_gen(
    comp_id: &str,
    frame: u64,
    scale: f32,
    generation: u64,
) -> Option<(u32, u32, Vec<u8>)> {
    let comp = Uuid::parse_str(comp_id).ok()?;
    // Take a cheap snapshot (an `Arc<Document>` clone) under the document lock;
    // its identity is this render's cache epoch. Released before the GPU work.
    let doc = with_bridge(|b| b.store.snapshot());
    // The comp's own frame rate, for the realtime controller's frame budget.
    let fps = doc.comp(comp).map(|c| c.frame_rate.fps()).unwrap_or(0.0);
    let key = crate::framecache::FrameKey::new(comp, frame, scale);
    crate::framecache::get_or_render(&doc, key, || {
        // A genuine miss: only render if this generation is still the latest
        // wanted — the stale-request skip (checked once the renderer lock is in
        // hand, the granularity the monolithic headless render allows).
        // Generation 0 is the legacy, non-cancellable entry point (the old FFI
        // call): it always renders and never touches the high-water mark.
        if generation != 0 && !crate::cancel::should_render(generation) {
            return None;
        }
        // Measure the wall-clock cost of this *genuine* render (a cache hit
        // never reaches here) and report it to the realtime tier controller
        // (K-171). `observe` only feeds the cost when `scale` matches the
        // controller's own tier — an Auto-mode render — so a manual-scale
        // render never corrupts the model.
        let started = std::time::Instant::now();
        let rendered = with_ready(|renderer| {
            renderer
                .render_rgba(&doc, comp, frame, scale)
                .ok()
                .map(|(rgba, w, h)| (w, h, rgba))
        })
        .flatten();
        if rendered.is_some() && fps > 0.0 {
            crate::realtime::observe(started.elapsed().as_secs_f64(), fps, scale);
        }
        rendered
    })
}

/// Render `comp_id` at `frame` under the ACTIVE transform preview (if any) —
/// the drag-preview sibling of [`render_comp_frame`]. Deliberately bypasses
/// [`crate::framecache`] entirely: the cache pins to the real document's
/// `Arc` identity, and a throwaway preview `Document` must never be pinned
/// there — doing so would clear the WHOLE cache every drag tick, exactly the
/// defect this fast path removes. Every call renders fresh; the caller
/// (Dart's `_pendingKey` latest-wins guard) is responsible for not calling
/// this more than once per outstanding frame. `None` on any failure (an
/// unknown/invalid comp id, no GPU adapter, or a render error) — the same
/// calm-null contract as `render_comp_frame`.
pub(crate) fn render_preview_frame(
    comp_id: &str,
    frame: u64,
    scale: f32,
) -> Option<(u32, u32, Vec<u8>)> {
    let comp = Uuid::parse_str(comp_id).ok()?;
    let doc = with_bridge(|b| crate::state::snapshot_with_preview(b));
    with_ready(|renderer| {
        renderer
            .render_rgba(&doc, comp, frame, scale)
            .ok()
            .map(|(rgba, w, h)| (w, h, rgba))
    })
    .flatten()
}

/// Run `f` against the session-lifetime headless renderer, building it lazily on
/// first use. `None` when the machine has no GPU adapter (the renderer resolves
/// to `Failed` and stays there — a calm, permanent "no frame"). The renderer's
/// own lock serialises the call, separate from the document lock, so a slow
/// render or export prep never blocks an edit. Shared by the Viewer render path
/// and the export-input builder ([`with_export_inputs`]) so both drive the one
/// renderer and share its probe cache.
fn with_ready<R>(f: impl FnOnce(&mut lumit_ui::headless::HeadlessRenderer) -> R) -> Option<R> {
    let mutex = RENDERER.get_or_init(|| Mutex::new(Slot::Uninit));
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    if matches!(*guard, Slot::Uninit) {
        *guard = match lumit_ui::headless::HeadlessRenderer::new() {
            Ok(renderer) => Slot::Ready(Box::new(renderer)),
            Err(_) => Slot::Failed,
        };
    }
    let Slot::Ready(renderer) = &mut *guard else {
        return None;
    };
    Some(f(renderer))
}

/// Render composition `comp_id` at `frame` into the Windows shared GPU texture
/// (K-177), returning `(handle, width, height)` — the zero-copy sibling of
/// [`render_comp_frame`]. `None` on any failure (an unknown/invalid comp id, no
/// D3D12 adapter, or a D3D interop error), which the FFI turns into `false` so
/// Dart falls back to the read-back path. The handle is stable across frames
/// (the same texture is re-used) and changes only on a comp resize. Present only
/// in the opt-in shared-texture build on Windows.
#[cfg(all(windows, feature = "shared-texture"))]
pub(crate) fn render_to_shared(comp_id: &str, frame: u64) -> Option<(u64, u32, u32)> {
    let comp = Uuid::parse_str(comp_id).ok()?;
    let doc = with_bridge(|b| b.store.snapshot());
    with_ready(|renderer| {
        renderer
            .render_to_shared(&doc, comp, frame)
            .ok()
            .map(|info| (info.handle, info.width, info.height))
    })
    .flatten()
}

/// The DMA-BUF metadata one Linux zero-copy frame carries (K-177): the exported
/// fd, dimensions, stride, offset, DRM fourcc and modifier.
#[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
pub(crate) type DmabufFrame = (i32, u32, u32, u32, u32, u32, u64);

/// Render composition `comp_id` at `frame` into the Linux DMA-BUF GPU texture
/// (K-177), returning its fd + DRM metadata — the zero-copy sibling of
/// [`render_to_shared`] for Linux. `None` on any failure (an unknown/invalid comp
/// id, no Vulkan adapter, missing external-memory extensions, or a Vulkan error),
/// which the FFI turns into `false` so Dart falls back to the read-back path. The
/// fd is stable across frames (the same texture is re-used) and changes only on a
/// comp resize. Present only in the opt-in shared-texture-linux build on Linux.
#[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
pub(crate) fn render_to_shared_dmabuf(comp_id: &str, frame: u64) -> Option<DmabufFrame> {
    let comp = Uuid::parse_str(comp_id).ok()?;
    let doc = with_bridge(|b| b.store.snapshot());
    with_ready(|renderer| {
        renderer
            .render_to_shared_dmabuf(&doc, comp, frame)
            .ok()
            .map(|info| {
                (
                    info.fd,
                    info.width,
                    info.height,
                    info.stride,
                    info.offset,
                    info.drm_fourcc,
                    info.modifier,
                )
            })
    })
    .flatten()
}

/// Compute a scope trace (waveform/vectorscope/histogram, K-096 v1) for the
/// frame the Viewer shows — `comp_id` at `frame`, at the same `scale` — and
/// return the `256×256` RGBA8 trace bytes. `kind` is `0` luma / `1` RGB waveform
/// / `2` vectorscope / `3` histogram; `colours` is `[bg, trace, red, green,
/// blue]` RGB byte triples (the frontend's fixed `ScopeColours`).
///
/// It rides the rendered-frame cache: the comp frame is fetched through
/// [`render_comp_frame`], so a frame already banked for the Viewer serves the
/// scope *without re-rendering the comp*, and the scope always traces the exact
/// bytes the Viewer shows (preview == the traced frame). Only the tiny trace is
/// read back — the heavy binning runs on the GPU. `None` on any failure (bad
/// comp id, unknown kind, no adapter, a render error).
pub(crate) fn render_scope(
    kind: u32,
    comp_id: &str,
    frame: u64,
    scale: f32,
    colours: [[u8; 3]; 5],
) -> Option<Vec<u8>> {
    // Serve the comp frame from the cache (or render it once) — the same key the
    // Viewer uses, so the scope and the picture never disagree.
    let (w, h, rgba) = render_comp_frame(comp_id, frame, scale)?;
    with_ready(|renderer| renderer.render_scope(&rgba, w, h, kind, colours).ok()).flatten()
}

/// Build the footage/audio inputs and a GPU export context for `comp` through
/// the headless seam (K-175), so the export driver can hand them to the exact
/// egui exporter (`lumit_ui::export::start`). `None` when the machine has no GPU
/// adapter or the comp is unknown. Reuses the same renderer instance the Viewer
/// path uses, so probes are shared and warm.
pub(crate) fn with_export_inputs(
    doc: &lumit_core::model::Document,
    comp: Uuid,
) -> Option<lumit_ui::headless::ExportInputs> {
    with_ready(|renderer| renderer.export_inputs(doc, comp)).flatten()
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

    /// The shared-texture path is `None` for a bad or unknown comp id, never a
    /// panic — the null path the FFI turns into `false` for Dart's fallback.
    /// (A real shared render against a live adapter is exercised in `lumit-ui`'s
    /// headless tests, which own a synthetic document; the bridge's global
    /// document is empty here.)
    #[cfg(all(windows, feature = "shared-texture"))]
    #[test]
    fn shared_render_of_a_bad_or_unknown_comp_is_none() {
        assert!(render_to_shared("not-a-uuid", 0).is_none());
        let unknown = Uuid::now_v7().to_string();
        assert!(render_to_shared(&unknown, 0).is_none());
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

    /// The preview render's null contract mirrors `render_comp_frame` exactly:
    /// a bad or unknown comp id is `None` before/without any GPU work needed,
    /// never a panic.
    #[test]
    fn preview_render_of_a_bad_or_unknown_comp_is_none() {
        assert!(render_preview_frame("not-a-uuid", 0, 1.0).is_none());
        let unknown = Uuid::now_v7().to_string();
        assert!(render_preview_frame(&unknown, 0, 1.0).is_none());
    }
}
