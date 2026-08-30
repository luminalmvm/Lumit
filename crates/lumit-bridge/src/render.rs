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
    let rung = preview_rung(sane);
    lumit_render::Quality {
        draft: false,
        auto_res: rung < 1.0,
        display_scale: rung,
        divisor: 1,
    }
}

/// How many steps the preview scale is allowed to take between "nothing" and
/// comp resolution.
///
/// Thirty-two, so a step is at most 3 % of the picture's width — finer than the
/// eye can tell from the step below it, and coarse enough that the whole range
/// is a short list.
const PREVIEW_RUNGS: f32 = 32.0;

/// The scale a preview is actually rendered at, given the one the Viewer asked
/// for: the next rung **up**, never down, so the picture is never softer than
/// what was asked for.
///
/// # Why a ladder at all
///
/// The Viewer reports the fraction of comp resolution its panel can show, and
/// that fraction is continuous — dragging a dock seam walks it through a new
/// value on every layout, dozens a second. Each distinct value is a differently
/// sized composite, and on the zero-copy path a differently sized composite is
/// a **new shared texture with a new handle**: the renderer mints one, the
/// frontend has to register it with the platform over a round trip, the old
/// registration is torn down, and every present in between makes the render
/// thread wait for the graphics card twice (see `SharedTexture::present`).
///
/// One handle per layout is the registration storm `lumit-render`'s
/// `shared_present` tests were written for. The target pool there stopped the
/// storm for sizes that *alternate* — one comp's renders finishing while
/// another's begin — but a seam drag does not alternate, it walks, and a walk
/// mints a fresh handle at every step no pool can hold. That is what made
/// dragging the Viewer's split flicker and then take the whole editor down with
/// it: the wait piles up, the frame stops arriving, and the driver eventually
/// resets the device out from under the process.
///
/// Snapping ends the walk. A drag now crosses a handful of rungs instead of one
/// value per pixel of pointer movement, the rungs repeat as the pointer moves
/// back, and the pool holds them. Two things fall out of it for free: a rung is
/// an exact binary fraction, so the same panel size always names the same
/// texture; and the frame cache keys on this scale (`preview_scale_q`), so
/// resizing a panel no longer throws away every frame already rendered.
///
/// It is deliberately *not* the playback tier ladder (`realtime::tier_scale`).
/// That one is four rungs, chosen to shed cost when a run cannot keep time; this
/// one is about how many *different* pictures a session asks for, and it must
/// stay fine enough that a Viewer docked small is still cheap to draw.
pub(crate) fn preview_rung(scale: f32) -> f32 {
    ((scale * PREVIEW_RUNGS).ceil() / PREVIEW_RUNGS).clamp(1.0 / PREVIEW_RUNGS, 1.0)
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
    // A device that has been lost takes this renderer with it (K-585). The
    // worker rebuilds its own; this one is rebuilt by putting the slot back to
    // where the first call finds it, which is the same road and no new one.
    // Without this, one driver reset would leave the export path holding a dead
    // renderer for the rest of the session, silently, while the Viewer beside
    // it recovered.
    if matches!(&*guard, Slot::Ready(renderer) if renderer.device_lost()) {
        *guard = Slot::Uninit;
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{preview_rung, quality_for, PREVIEW_RUNGS};

    /// A rung is never below what was asked for, so snapping can only ever make
    /// the picture sharper than the request, never softer.
    #[test]
    fn a_rung_is_never_below_the_scale_it_was_asked_for() {
        let mut scale = 0.001_f32;
        while scale <= 1.0 {
            let rung = preview_rung(scale);
            assert!(
                rung >= scale,
                "{scale} snapped down to {rung}, which is a softer picture than \
                 was asked for"
            );
            assert!(rung <= 1.0, "{scale} snapped above comp resolution");
            scale += 0.0007;
        }
        assert_eq!(preview_rung(1.0), 1.0);
        assert_eq!(preview_rung(0.5), 0.5);
        // Nothing may reach zero: a zero-sized composite is a zero-sized
        // texture, and no graphics API will make one.
        assert!(preview_rung(f32::MIN_POSITIVE) > 0.0);
    }

    /// The same panel size always names the same picture. Rungs are exact
    /// binary fractions, so this is equality and not a tolerance — the frame
    /// cache keys on the number, and two names for one picture is a miss.
    #[test]
    fn one_rung_is_one_number() {
        for k in 1..=PREVIEW_RUNGS as u32 {
            let exact = k as f32 / PREVIEW_RUNGS;
            assert_eq!(preview_rung(exact), exact);
            // A hair under lands on the same rung; a hair over is the next one.
            assert_eq!(preview_rung(exact - 0.001), exact);
        }
    }

    /// **The regression.** A dock seam being dragged reports a new fraction on
    /// every layout — hundreds of them across one gesture. Each distinct one is
    /// a differently sized composite, and on the zero-copy path each is a fresh
    /// shared-texture handle the frontend has to register with the platform.
    /// That storm is what took the editor down; the ladder is what bounds it.
    #[test]
    fn a_seam_drag_asks_for_a_handful_of_sizes_not_one_per_layout() {
        // A Viewer growing from a third of an HD comp to two thirds, a pixel of
        // pointer movement at a time — the drag that was reported.
        let asked: Vec<f32> = (0..640).map(|i| 0.33 + i as f32 * 0.0005).collect();
        let mut raw: Vec<u32> = asked.iter().map(|s| (s * 1920.0).round() as u32).collect();
        raw.sort_unstable();
        raw.dedup();
        assert!(
            raw.len() > 200,
            "the drag itself has to be a storm, or this test proves nothing"
        );

        let mut rungs: Vec<u32> = asked
            .iter()
            .map(|s| (quality_for(*s).display_scale * 1920.0).round() as u32)
            .collect();
        rungs.sort_unstable();
        rungs.dedup();
        assert!(
            rungs.len() <= 12,
            "the drag asked the renderer for {} different sizes; a handle per \
             size is the registration storm this ladder exists to stop",
            rungs.len()
        );
    }

    /// And over a whole session, however the panels are moved about, there are
    /// only ever as many sizes as there are rungs.
    #[test]
    fn the_whole_range_is_a_short_list() {
        let mut seen: Vec<u32> = (0..10_000)
            .map(|i| quality_for(i as f32 / 10_000.0).display_scale)
            .map(|s| (s * PREVIEW_RUNGS).round() as u32)
            .collect();
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() <= PREVIEW_RUNGS as usize, "{} rungs", seen.len());
    }

    /// A scale that means nothing still renders something, and at full.
    #[test]
    fn a_nonsense_scale_falls_back_to_comp_resolution() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let q = quality_for(bad);
            assert_eq!(q.display_scale, 1.0);
            assert!(!q.auto_res);
        }
    }
}
