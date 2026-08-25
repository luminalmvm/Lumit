//! The realtime preview-tier controller, wired into the Viewer render path
//! (K-030/K-171).
//!
//! # In plain terms
//!
//! During playback the machine may not keep up at full resolution. The realtime
//! controller watches how long frames actually take and, when they get too slow
//! for the frame budget, drops the preview to a coarser resolution (Full → Half
//! → Third → Quarter); when frames are comfortably fast for a sustained stretch
//! it earns the resolution back — quick to worsen, slow to improve, so the
//! picture never flickers between qualities. That decision core is
//! [`lumit_eval::schedule::RealtimeController`], already built and tested; it was
//! historically unwired (K-171). This module runs one instance for the session
//! and connects it to the bridge's pull-model rendering.
//!
//! The fit to the pull model: the Viewer render path measures the wall-clock
//! cost of each *genuine* GPU render (a cache hit is not a render, so it is not
//! measured) and reports it here via [`observe`]. The controller updates its
//! smoothed cost and picks the tier. Dart reads the tier back with
//! [`current`] (exposed as the `playback_tier` op) to show the Viewer readout
//! and, in **Auto** resolution mode, to choose the scale for the next frame.
//!
//! Manual override (the resolution picker set to Half/Third/Quarter rather than
//! Auto) is a Dart-side decision: Dart simply passes its chosen scale to the
//! render and ignores the tier. So it does not corrupt the controller, [`observe`]
//! only feeds a cost when the render was issued at the controller's *own* tier
//! scale (an Auto render). A manual render at a different scale is measured for
//! nothing — the controller keeps modelling the Auto tier. [`reset`] restarts the
//! controller (Dart calls it when playback stops, the comp changes, or the user
//! switches back to Auto), so a fresh run starts optimistic at Full.

use lumit_eval::schedule::{RealtimeController, COARSEST_TIER, FINEST_TIER};
use std::sync::{Mutex, OnceLock};

/// The session-lifetime controller behind its own lock (independent of the
/// document and renderer locks — reading the tier never blocks an edit).
static CONTROLLER: OnceLock<Mutex<RealtimeController>> = OnceLock::new();

fn with_controller<R>(f: impl FnOnce(&mut RealtimeController) -> R) -> R {
    let mutex = CONTROLLER.get_or_init(|| Mutex::new(RealtimeController::new()));
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    f(&mut guard)
}

/// The preview divisor for a tier (1 = Full, 2 = Half, 3 = Third, 4 = Quarter)
/// as a render scale (`1.0 / tier`). The one mapping Dart and the render path
/// share, so "am I rendering at the controller's tier?" is one comparison.
pub(crate) fn tier_scale(tier: u32) -> f32 {
    1.0 / tier.clamp(FINEST_TIER, COARSEST_TIER) as f32
}

/// The most raster pixels a live-drag preview frame is allowed to cover
/// (640x360). A drag tick has to land inside budget B3 of
/// docs/13-PERFORMANCE-RULES.md — 50 ms from the input to a possibly degraded
/// frame — and gather kernels (Depth of field, Lens dirt) cost roughly the
/// raster area times the square of their radius, and the radius is itself in
/// px@comp and so scales with the preview factor. So the saving from a coarser
/// drag raster is far steeper than the pixel count suggests: a third of the
/// scale is a ninth of the pixels and, for those two, something nearer a
/// hundredth of the work.
const DRAG_PIXEL_BUDGET: u64 = 640 * 360;

/// The tier a *live drag* renders at: the finest divisor (1 = Full … 4 =
/// Quarter) whose raster fits [`DRAG_PIXEL_BUDGET`], given the comp's pixel
/// size and the scale the Viewer is already asking for.
///
/// In plain terms: while a value is being dragged the picture is being read for
/// *shape* — is the blur too big, is the dirt too bright — and a soft answer
/// now beats a sharp one three seconds late. So a drag frame is rendered small.
/// Nothing is degraded that was already cheap: a small comp, or one shown in a
/// small panel, is under budget at Full and stays there. The full-resolution
/// frame comes back on release, when the commit lands and the ordinary render
/// path asks for the frame at the Viewer's own scale.
///
/// Deliberately *not* the adaptive [`RealtimeController`]: that one learns from
/// measured costs over a dozen frames, and a drag is over before it has
/// finished learning — the first drag on a heavy comp would still stall, which
/// is the whole complaint. This is a decision taken before the first tick.
pub(crate) fn drag_tier(width: u32, height: u32, scale: f32) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        f64::from(scale).min(1.0)
    } else {
        1.0
    };
    let area = f64::from(width) * f64::from(height) * scale * scale;
    for tier in FINEST_TIER..COARSEST_TIER {
        if area / f64::from(tier * tier) <= DRAG_PIXEL_BUDGET as f64 {
            return tier;
        }
    }
    COARSEST_TIER
}

/// The scale to render a live-drag frame at, given the scale the Viewer asked
/// for and the comp's pixel size. See [`drag_tier`].
pub(crate) fn drag_scale(width: u32, height: u32, scale: f32) -> f32 {
    scale * tier_scale(drag_tier(width, height, scale))
}

/// Report one genuine render's measured `cost_secs` at frame rate `fps`, but
/// only when it was issued at the controller's own tier scale (an Auto render) —
/// a manual render at a different `scale` is not the controller's business and
/// is ignored, so it cannot mislead the model. Returns the tier in force after
/// the report (unchanged on an ignored cost). Called only from the render path
/// (the `render` feature); the tier read-back ops compile in every build.
pub(crate) fn observe(cost_secs: f64, fps: f64, scale: f32) -> u32 {
    with_controller(|c| {
        let expected = tier_scale(c.tier());
        // A small tolerance: Dart's Auto scale is derived from the same
        // `tier_scale`, so an exact match is expected, but float equality is
        // fragile — accept anything within half a tier step.
        if (scale - expected).abs() <= 0.01 {
            c.record(cost_secs, fps)
        } else {
            c.tier()
        }
    })
}

/// The tier currently in force (1..=4).
pub(crate) fn tier() -> u32 {
    with_controller(|c| c.tier())
}

/// Restart the controller — optimistic at Full again. Called when playback
/// stops, the composition changes, or the user switches back to Auto, so a fresh
/// run does not inherit a stale tier.
pub(crate) fn reset() {
    with_controller(|c| *c = RealtimeController::new());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{drag_scale, drag_tier, DRAG_PIXEL_BUDGET};
    use lumit_eval::schedule::RealtimeController;

    /// A drag on a comp small enough to be cheap already is not degraded at all
    /// — softening a picture that was keeping up buys nothing and costs the
    /// only thing being judged.
    #[test]
    fn a_small_comp_drags_at_full_resolution() {
        assert_eq!(drag_tier(640, 360, 1.0), 1);
        assert_eq!(drag_scale(640, 360, 1.0), 1.0);
        // And a big comp shown in a small panel is small too: the Viewer's own
        // scale is already inside the budget, so there is nothing to take.
        assert_eq!(drag_tier(1920, 1080, 0.33), 1);
    }

    /// The case the owner hit: a 1080p comp at full scale, Depth of field on a
    /// layer, the picture 1-5 s behind the pointer. It has to come down.
    #[test]
    fn a_full_size_comp_drags_coarser() {
        assert_eq!(drag_tier(1920, 1080, 1.0), 3);
        assert!((drag_scale(1920, 1080, 1.0) - 1.0 / 3.0).abs() < 1e-6);
        // 4K goes to the floor rather than further: below Quarter the picture
        // stops being judgeable, which is the point of dragging it.
        assert_eq!(drag_tier(3840, 2160, 1.0), 4);
        assert_eq!(drag_tier(7680, 4320, 1.0), 4);
    }

    /// Whatever tier is picked, the raster it implies is inside the budget —
    /// unless the floor was hit, in which case it is the floor's fault and not
    /// the search's.
    #[test]
    fn the_chosen_tier_is_the_finest_one_inside_the_budget() {
        for &(w, h) in &[
            (16u32, 9u32),
            (640, 360),
            (1280, 720),
            (1920, 1080),
            (2048, 858),
            (3840, 2160),
        ] {
            for &scale in &[0.25f32, 0.5, 0.75, 1.0] {
                let tier = drag_tier(w, h, scale);
                let area =
                    f64::from(w) * f64::from(h) * f64::from(scale * scale) / f64::from(tier * tier);
                assert!(
                    area <= DRAG_PIXEL_BUDGET as f64 || tier == 4,
                    "{w}x{h} at {scale} chose tier {tier}, which is {area} pixels"
                );
                if tier > 1 {
                    let finer = tier - 1;
                    let bigger = f64::from(w) * f64::from(h) * f64::from(scale * scale)
                        / f64::from(finer * finer);
                    assert!(
                        bigger > DRAG_PIXEL_BUDGET as f64,
                        "{w}x{h} at {scale} could have stayed at tier {finer}"
                    );
                }
            }
        }
    }

    /// A nonsense scale must not produce a nonsense raster. Zero, negative and
    /// NaN all read as "full", which is what every other scale guard in the
    /// bridge does (`quality_for`), and over-1.0 is clamped so a zoomed Viewer
    /// cannot ask a drag to render bigger than the comp.
    #[test]
    fn a_nonsense_scale_is_treated_as_full() {
        for scale in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(drag_tier(1920, 1080, scale), 3, "scale {scale}");
        }
        assert_eq!(drag_tier(1920, 1080, 4.0), 3);
    }

    /// **The costs this controller is fed, written down.**
    ///
    /// Its own controller rather than the process-global one: the tier is shared
    /// session state and other tests reset it, so asserting exact tiers against
    /// the global would race. What is pinned here is that the numbers actually
    /// measured on the read-back transport reach a verdict, which is the thing
    /// that silently was not true.
    ///
    /// The render path used to stop its clock *before* handing the pixels to
    /// Dart, so it reported the render alone. Measured on this transport, a
    /// 1.44 MB frame (800x450) costs about 3 ms to render and about 6 ms to hand
    /// over — the hand-off is the larger half and is linear in bytes, so a full
    /// 1080p frame is around 35 ms against a 16.7 ms budget at 60 fps. Reporting
    /// 3 ms of that left the controller believing it had headroom, so it never
    /// left Full and playback skipped frames instead of getting softer.
    #[test]
    fn the_measured_read_back_costs_reach_the_right_verdicts() {
        // A full-size 1080p frame on this transport: hopeless at 60 fps.
        let mut over = RealtimeController::new();
        assert_eq!(over.tier(), 1, "a fresh controller is optimistic");
        assert!(
            over.record(0.035, 60.0) > 1,
            "35 ms a frame against a 16.7 ms budget must coarsen the preview, \
             not sit at Full while playback skips frames around it"
        );

        // The render cost alone, which is what used to be reported. It has to
        // read as comfortable — that is precisely why the old measurement never
        // moved anything, and why the fix is where the clock stops, not here.
        let mut render_only = RealtimeController::new();
        for _ in 0..20 {
            render_only.record(0.003, 60.0);
        }
        assert_eq!(
            render_only.tier(),
            1,
            "3 ms of a 16.7 ms budget is not a reason to soften — so a \
             controller fed only the render can never do its job"
        );
    }
}
