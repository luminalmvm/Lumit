//! The realtime preview-tier controller, wired into the Viewer render path.
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
//! historically unwired. This module runs one instance for the session
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

/// Whether the user has asked for live drags to be rendered at the Viewer's own
/// resolution — Settings → Performance, and off by default.
///
/// The switch exists because [`DRAG_PIXEL_BUDGET`] is a trade, not a truth: it
/// buys a picture that keeps up with the pointer by giving up sharpness, and
/// somebody with a fast card and a light composition is paying the price
/// without needing to. Off is still the default, because the machine that
/// cannot afford full-resolution drags is the one that cannot tell you so.
///
/// An atomic rather than a seat in the controller's lock: it is read on every
/// drag frame and written twice a session, and it has nothing to do with what
/// the controller is measuring.
static FULL_RES_DRAGS: AtomicBool = AtomicBool::new(false);

/// Set the [`FULL_RES_DRAGS`] preference. Called at boot with what the settings
/// file held, and again whenever the switch is thrown.
pub(crate) fn set_full_res_drags(on: bool) {
    FULL_RES_DRAGS.store(on, Ordering::Relaxed);
}

/// The scale to render a live-drag frame at, given the scale the Viewer asked
/// for and the comp's pixel size. See [`drag_tier`].
///
/// With [`FULL_RES_DRAGS`] on this is the Viewer's own scale unchanged — the
/// drag budget is the only thing being turned off, so a drag then renders
/// exactly as a committed frame does, and takes exactly as long.
pub(crate) fn drag_scale(width: u32, height: u32, scale: f32) -> f32 {
    let full_res = FULL_RES_DRAGS.load(Ordering::Relaxed);
    if full_res {
        return scale;
    }
    let tier = drag_tier(width, height, scale);
    let divisor = adaptive_divisor(
        tier,
        DRAG_EXTRA.load(Ordering::Relaxed),
        DRAG_LIMIT.load(Ordering::Relaxed),
    );
    scale / divisor as f32
}

/// **Adaptive resolution limit** (Settings → Performance), as After Effects
/// CS6 had it: how coarse a drag may go when even the drag budget's tier is
/// too slow. 0 is Auto, which goes as far as an eighth; 2, 4, 8 and 16 cap the
/// divisor there.
static DRAG_LIMIT: AtomicU32 = AtomicU32::new(0);

/// How many times coarser than the drag budget's tier the drags are drawing
/// now, learnt from what they cost: doubled while a drag frame takes longer
/// than [`SLOW_DRAG`], halved back once one takes under [`QUICK_DRAG`]. 1 is
/// the budget's tier itself, which is where every machine fast enough for it
/// stays, so a drag there looks exactly as it did before.
static DRAG_EXTRA: AtomicU32 = AtomicU32::new(1);

/// A drag frame slower than this coarsens the next one: the 50 ms a drag
/// tick has (docs/13 §2, budget B3).
const SLOW_DRAG: f64 = 0.050;
/// A drag frame quicker than this sharpens the next one back.
const QUICK_DRAG: f64 = 0.020;

/// The coarsest divisor Auto goes to.
const AUTO_LIMIT: u32 = 8;

/// Set the adaptive resolution limit: 0 for Auto, else 2, 4, 8 or 16. A new
/// limit starts the learning again.
pub(crate) fn set_drag_limit(divisor: u32) {
    let divisor = match divisor {
        0 => 0,
        d => d.clamp(2, 16),
    };
    DRAG_LIMIT.store(divisor, Ordering::Relaxed);
    DRAG_EXTRA.store(1, Ordering::Relaxed);
}

/// The divisor a drag frame is drawn at: the budget's `tier` times what was
/// learnt (`extra`), capped by `limit` (0 for Auto). A manual limit finer
/// than the budget's tier is taken as asked: the drag is then sharper and
/// slower, which is what choosing it says.
fn adaptive_divisor(tier: u32, extra: u32, limit: u32) -> u32 {
    let cap = if limit == 0 {
        AUTO_LIMIT.max(tier)
    } else {
        limit
    };
    tier.saturating_mul(extra.max(1)).clamp(1, cap.max(1))
}

/// Report what a drag frame cost, so the next one is drawn coarser while
/// drags are slow and finer again once they are quick. Ignored with full
/// resolution drags on, which ask for the Viewer's own scale whatever it
/// costs.
pub(crate) fn observe_drag(cost_secs: f64) {
    if FULL_RES_DRAGS.load(Ordering::Relaxed) {
        return;
    }
    let extra = DRAG_EXTRA.load(Ordering::Relaxed);
    let next = next_extra(extra, cost_secs);
    DRAG_EXTRA.store(next, Ordering::Relaxed);
}

/// [`observe_drag`]'s rule: double while slow, halve while quick, never under
/// 1 and never past 16 (the cap in [`adaptive_divisor`] does the rest).
fn next_extra(extra: u32, cost_secs: f64) -> u32 {
    if cost_secs > SLOW_DRAG {
        (extra.max(1) * 2).min(16)
    } else if cost_secs < QUICK_DRAG {
        (extra / 2).max(1)
    } else {
        extra.max(1)
    }
}

/// **Wireframe while dragging** (Settings → Performance, off by default): a
/// drag frame draws every layer as the outline of its rectangle, with no
/// effects, styles, mattes or motion blur, placed exactly as the frame places
/// it, and the release draws the real frame. After Effects CS6's Wireframe
/// fast preview.
static WIREFRAME_DRAGS: AtomicBool = AtomicBool::new(false);

/// **Live Update** (the Viewer's switch, on by default): off, a drag draws no
/// frame at all, and the Viewer keeps the last one until the release commits,
/// as After Effects' Live Update switch.
static LIVE_UPDATE: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_wireframe_drags(on: bool) {
    WIREFRAME_DRAGS.store(on, Ordering::Relaxed);
}

pub(crate) fn wireframe_drags() -> bool {
    WIREFRAME_DRAGS.load(Ordering::Relaxed)
}

pub(crate) fn set_live_update(on: bool) {
    LIVE_UPDATE.store(on, Ordering::Relaxed);
}

pub(crate) fn live_update() -> bool {
    LIVE_UPDATE.load(Ordering::Relaxed)
}

/// Strip a drag frame's document down to what a wireframe shows: no effect,
/// style, matte or motion blur on any layer, every layer blended Normal, so
/// each one's outline is drawn where it sits and nothing hides or gates it.
/// The document is the drag's own clone; the project is untouched.
pub(crate) fn wireframe_document(document: &mut lumit_core::model::Document) {
    for item in &mut document.items {
        let lumit_core::model::ProjectItem::Composition(comp) = item else {
            continue;
        };
        for layer in &mut comp.layers {
            for fx in layer.effects.iter_mut().chain(layer.styles.iter_mut()) {
                fx.enabled = false;
            }
            layer.matte = None;
            layer.switches.motion_blur = false;
            layer.blend = lumit_core::model::BlendMode::default();
        }
    }
}

/// Drop every Lens flare in `document` to Draft for a drag frame. The flare
/// renders about 6 times faster there, and the committed frame after the drag
/// is drawn at its own quality. Left alone with full-resolution drags, which
/// render a drag exactly as a committed frame.
pub(crate) fn draft_flares(document: &mut lumit_core::model::Document) {
    if FULL_RES_DRAGS.load(Ordering::Relaxed) {
        return;
    }
    for item in &mut document.items {
        let lumit_core::model::ProjectItem::Composition(comp) = item else {
            continue;
        };
        for fx in comp.effects_mut() {
            if fx.effect.match_name != "lens_flare" {
                continue;
            }
            if let Some(quality) = fx.params.iter_mut().find(|p| p.id == "quality") {
                quality.value = lumit_core::model::EffectValue::Choice(0);
            }
        }
    }
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
    use super::{adaptive_divisor, drag_tier, next_extra, DRAG_PIXEL_BUDGET};

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

    /// A machine fast enough for the drag budget stays at its tier; a slow
    /// one goes coarser up to the limit, and comes back once drags are quick.
    #[test]
    fn slow_drags_go_coarser_up_to_the_limit_and_come_back() {
        assert_eq!(
            adaptive_divisor(2, 1, 0),
            2,
            "nothing learnt: the budget's tier"
        );
        assert_eq!(adaptive_divisor(2, 2, 0), 4);
        assert_eq!(adaptive_divisor(2, 16, 0), 8, "Auto stops at an eighth");
        assert_eq!(adaptive_divisor(2, 16, 16), 16);
        assert_eq!(adaptive_divisor(3, 4, 4), 4, "a manual limit caps it");
        assert_eq!(
            adaptive_divisor(3, 1, 2),
            2,
            "a finer manual limit is taken as asked"
        );

        assert_eq!(next_extra(1, 0.080), 2);
        assert_eq!(next_extra(8, 0.080), 16);
        assert_eq!(next_extra(16, 0.080), 16);
        assert_eq!(next_extra(4, 0.030), 4, "between the two: stay");
        assert_eq!(next_extra(4, 0.010), 2);
        assert_eq!(next_extra(1, 0.010), 1);
    }
}
