//! Cutting the shot up: which tracks belong to something moving by itself, and
//! where the lens changed (docs/impl/tracking.md §3).
//!
//! # In plain terms
//!
//! Two jobs live here, and they are the same shape: look at a number over the
//! length of the shot, and say where it changes character.
//!
//! The first number is, for one followed speck, how far it sits from where the
//! camera's own geometry says it should be. A speck on a parked car agrees with
//! that geometry on every pair of keyframes it survives. A speck on a moving car
//! disagrees on every one, and is marked [`crate::TrackState::Moving`] so the
//! solve never sees it — the point is kept, because that is exactly the track an
//! object solve wants later. The interesting middle case is a speck on a car
//! that was parked and then drove off: it agrees for a while and then stops
//! agreeing, and the honest reading is not "this track is bad" but "this is two
//! tracks". It is **split** at the change, the front half stays in the solve
//! under its own id, and the back half becomes a new track that remembers which
//! one it came from.
//!
//! The second number is how much everything in the frame grew between two
//! adjacent frames — the median of the scale the tracker already measured for
//! every patch it followed ([`crate::TrackSet::median_log_scale`]). Still
//! camera, still lens: zero. A smooth zoom: a small steady value for as long as
//! the operator holds the ring — a **ramp**. The owner's scope-in, where the
//! focal length leaps between two frames: one enormous value and nothing on
//! either side — a **cut**. The two are told apart by how long the run is and by
//! one cross-check: in a genuine zoom cut the whole frame is explained by a
//! scale about a single centre and nothing else, so a scale-only fit to the
//! displacements has to leave almost no residual. A camera lunging forward moves
//! near things more than far ones and fails that check, which is the point of
//! making it.

use crate::geom::{self, sampson_distance};
use crate::pairs::PairGeometry;
use crate::{Correspondence, TrackSet, TrackState};

/// The knobs the dynamic-track segmentation takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentSettings {
    /// Epipolar (Sampson) residual in source pixels above which a track is
    /// judged to disagree with the pair's dominant model. Deliberately looser
    /// than the RANSAC inlier threshold: this is a verdict about a track's
    /// whole life, and a marginal frame should not start it.
    pub residual_px: f64,
    /// Keyframe pairs a track must span before it is judged at all. One pair is
    /// an opinion, not a profile.
    pub min_pairs: usize,
    /// Fraction of the spanned pairs that must disagree for the whole track to
    /// be called moving.
    pub dirty_fraction: f64,
}

impl Default for SegmentSettings {
    fn default() -> Self {
        SegmentSettings {
            residual_px: 3.0,
            min_pairs: 2,
            dirty_fraction: 0.8,
        }
    }
}

/// Where one track was cut in two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackSplit {
    /// The original track, which keeps its id and its points up to and
    /// including `at_frame`.
    pub parent: u32,
    /// The new track carrying the points from `at_frame + 1` onwards.
    pub child: u32,
    /// The last frame the parent still agreed with the camera on.
    pub at_frame: i64,
}

/// What the segmentation did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segmentation {
    /// Every track now in state [`crate::TrackState::Moving`], ascending —
    /// including the children of splits.
    pub moving: Vec<u32>,
    /// Every cut made, in the order they were made.
    pub splits: Vec<TrackSplit>,
}

/// Judge every track against the dominant models of the keyframe pairs it
/// spans, mark the ones that never agreed as moving, and split the ones that
/// stopped agreeing (docs/impl/tracking.md §3).
///
/// `pairs` must be in frame order — [`crate::select_keyframes`] returns them
/// that way — because "clean prefix, dirty suffix" is a statement about time.
pub fn segment_dynamic_tracks(
    set: &mut TrackSet,
    pairs: &[PairGeometry],
    settings: &SegmentSettings,
) -> Segmentation {
    enum Decision {
        Moving,
        SplitAt(i64),
    }

    // Decide first, mutate second. A split appends a track, and a decision
    // taken against a half-mutated store would be read off the wrong points.
    let mut decisions: Vec<(u32, Decision)> = Vec::new();
    let mut profile: Vec<(i64, bool)> = Vec::new();
    for track in set.tracks() {
        profile.clear();
        for g in pairs {
            let (Some(a), Some(b)) = (track.point_at(g.from), track.point_at(g.to)) else {
                continue;
            };
            let r = sampson_distance(&g.fundamental, [a.x, a.y], [b.x, b.y]);
            profile.push((g.from, !r.is_finite() || r > settings.residual_px));
        }
        if profile.len() < settings.min_pairs.max(1) {
            continue;
        }
        let dirty = profile.iter().filter(|p| p.1).count();
        if dirty == 0 {
            continue;
        }
        let Some(first_dirty) = profile.iter().position(|p| p.1) else {
            continue;
        };
        if first_dirty == 0 && dirty as f64 >= settings.dirty_fraction * profile.len() as f64 {
            decisions.push((track.id, Decision::Moving));
        } else if first_dirty > 0 && profile.iter().skip(first_dirty).all(|p| p.1) {
            if let Some(&(at, _)) = profile.get(first_dirty) {
                decisions.push((track.id, Decision::SplitAt(at)));
            }
        }
    }

    let mut out = Segmentation::default();
    for (id, decision) in decisions {
        match decision {
            Decision::Moving => {
                if set.mark_moving(id) {
                    out.moving.push(id);
                }
            }
            Decision::SplitAt(at) => {
                if let Some(child) = set.split_track(id, at) {
                    set.mark_moving(child);
                    out.moving.push(child);
                    out.splits.push(TrackSplit {
                        parent: id,
                        child,
                        at_frame: at,
                    });
                }
            }
        }
    }
    out.moving.sort_unstable();
    out
}

// --- The zoom-burst detector ------------------------------------------------

/// The knobs the zoom detector takes (docs/impl/tracking.md §3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomSettings {
    /// `|log scale|` per frame pair above which the pair's growth is worth
    /// judging at all. Below this is tracker noise.
    pub ramp_threshold: f64,
    /// Excess `|log scale|` over the travel baseline a single isolated pair
    /// must exceed before it can be a cut rather than a one-frame ramp. A
    /// scope-in is an enormous number here; an operator's hand is not.
    pub cut_threshold: f64,
    /// Residual, in source pixels, that a scale-only fit to the pair's
    /// displacements must stay under for the pair to be a cut. This is what
    /// separates "the lens changed" from "the camera lunged".
    pub scale_only_px: f64,
    /// How far the scale-only fit's own `log scale` may differ from the median
    /// of the tracker's affine matrices before the two are not describing the
    /// same event. The note's cross-check, made a number.
    pub cross_check: f64,
    /// The fraction of a pair's radial-scale displacement its scale-only
    /// residual may reach before the growth is read as travel rather than
    /// lens. A zoom is a scale about one centre and leaves only noise behind;
    /// a dolly moves near things more than far ones, and that parallax is a
    /// roughly constant *fraction* of the flow however slow the travel is —
    /// which is what makes the fraction, unlike an absolute threshold,
    /// speed-independent.
    pub parallax_fraction: f64,
    /// The radial-scale displacement, in source pixels, the classification
    /// needs before the parallax fraction means anything. A pair too slow to
    /// reach it on its own is judged over a widening window of its neighbours
    /// instead — parallax accumulates coherently with travel while tracker
    /// noise does not, so pooling the pairs is what makes a slow dolly and a
    /// slow zoom distinguishable at all.
    pub signature_px: f64,
    /// How many pairs either side the classification window may grow to. A
    /// pair still too slow to judge at this width contributes to the travel
    /// baseline rather than to any boundary.
    pub signature_window: i64,
    /// Pairs either side over which the travel baseline is taken — the median
    /// `log scale` of the pairs the signature read as camera motion. A lens
    /// event is an excess above this, so a cut inside a forward-moving shot is
    /// judged against its neighbours rather than against zero.
    pub baseline_window: i64,
}

impl Default for ZoomSettings {
    fn default() -> Self {
        ZoomSettings {
            ramp_threshold: 0.004,
            cut_threshold: 0.05,
            scale_only_px: 1.5,
            cross_check: 0.05,
            parallax_fraction: 0.15,
            signature_px: 4.0,
            signature_window: 6,
            baseline_window: 12,
        }
    }
}

/// What kind of focal change a boundary is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomKind {
    /// Focal leapt between two adjacent frames — the owner's scope-in. Pose is
    /// continuous across it and focal is free: a segment boundary.
    Cut,
    /// Focal moved smoothly over a run of pairs. Not a boundary between
    /// segments but a stretch within one, where focal is a smooth function
    /// rather than a constant.
    Ramp,
}

/// One focal-change segment boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomBoundary {
    /// The first pair of the run: the change happens between `frame` and
    /// `frame + 1`.
    pub frame: i64,
    /// The last pair of the run. Equal to `frame` for a [`ZoomKind::Cut`].
    pub end_frame: i64,
    pub kind: ZoomKind,
    /// Median `log scale` over the run — `ln 1.4` for a 1.4× scope-in.
    pub log_scale: f64,
}

/// Find the focal-change boundaries in a track set (docs/impl/tracking.md §3).
///
/// A pair is never judged against zero: each hot pair is first classified by
/// the radial-flow-versus-parallax signature — a zoom is a scale about one
/// centre and leaves only noise behind its scale-only fit, while travel moves
/// near things more than far ones and leaves a residual that is a roughly
/// constant fraction of the flow. Pairs too slow to carry the signature on
/// their own are judged over a widening window of their neighbours, because
/// parallax accumulates coherently with travel and tracker noise does not.
/// The pairs the signature reads as travel form a per-pair **baseline** (a
/// windowed median), and a boundary is an *excess* of lens-like growth above
/// it — which is what lets a scope-in survive inside a forward-moving shot
/// instead of disappearing into the shot's own growth.
///
/// Returned in frame order.
#[must_use]
pub fn detect_zoom(set: &TrackSet, settings: &ZoomSettings) -> Vec<ZoomBoundary> {
    let mut out = Vec::new();
    let Some((first, last)) = set.frame_range() else {
        return out;
    };
    if last <= first {
        return out;
    }
    let pairs = usize::try_from(last - first).unwrap_or(0);

    // 1. Every pair's growth, once.
    let m: Vec<Option<f64>> = (0..pairs)
        .map(|i| set.median_log_scale(first + i as i64))
        .collect();

    // 2. The signature: is this hot pair's growth lens or travel? `None` is a
    //    cold pair (or one with no data), which is neither.
    let lens: Vec<Option<bool>> = m
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let v = (*v)?;
            if v.abs() <= settings.ramp_threshold {
                return None;
            }
            Some(lens_signature(set, first + i as i64, first, last, settings))
        })
        .collect();

    // 3. The travel baseline: per pair, the median growth of the pairs around
    //    it that the signature did *not* read as lens — the shot's own motion,
    //    which a lens event has to stand above. Zero where nothing nearby says
    //    otherwise, which keeps a still or purely zooming shot exactly where
    //    the old thresholds put it.
    let window = usize::try_from(settings.baseline_window.max(0)).unwrap_or(0);
    let mut scratch: Vec<f64> = Vec::with_capacity(2 * window + 1);
    let baseline: Vec<f64> = (0..pairs)
        .map(|i| {
            scratch.clear();
            let lo = i.saturating_sub(window);
            let hi = (i + window).min(pairs.saturating_sub(1));
            for j in lo..=hi {
                if lens.get(j).copied().flatten() != Some(true) {
                    if let Some(Some(v)) = m.get(j) {
                        scratch.push(*v);
                    }
                }
            }
            geom::median(&mut scratch).unwrap_or(0.0)
        })
        .collect();

    // 4. Runs of lens-like excess. A pair enters a run only when the signature
    //    called it lens *and* its excess over the baseline clears the ramp
    //    threshold — travel pairs bound the runs instead of joining them.
    let excess = |i: usize| -> Option<f64> {
        if lens.get(i).copied().flatten() != Some(true) {
            return None;
        }
        let e = (*m.get(i)?)? - baseline.get(i).copied().unwrap_or(0.0);
        (e.abs() > settings.ramp_threshold).then_some(e)
    };
    let mut i = 0usize;
    while i < pairs {
        let Some(head) = excess(i) else {
            i += 1;
            continue;
        };
        let mut run = vec![head];
        let mut end = i;
        while end + 1 < pairs {
            let Some(e) = excess(end + 1) else {
                break;
            };
            run.push(e);
            end += 1;
        }
        let frame = first + i as i64;
        let log_scale = geom::median(&mut run).unwrap_or(head);
        let kind = if end == i
            && head.abs() > settings.cut_threshold
            && scale_only(
                set,
                frame,
                m.get(i).copied().flatten().unwrap_or(head),
                settings,
            ) {
            ZoomKind::Cut
        } else {
            ZoomKind::Ramp
        };
        out.push(ZoomBoundary {
            frame,
            end_frame: first + end as i64,
            kind,
            log_scale,
        });
        i = end + 1;
    }
    out
}

/// Whether the growth on the pair `frame → frame + 1` is a lens event rather
/// than travel, read from the radial-flow-versus-parallax signature: the
/// scale-only fit's residual as a fraction of the radial displacement the
/// scale itself accounts for. The pair is pooled with up to
/// [`ZoomSettings::signature_window`] neighbours either side until the
/// displacement reaches [`ZoomSettings::signature_px`]; a pair still too slow
/// to judge at full width is travel's to keep — it can always join the
/// baseline, but a boundary needs evidence.
fn lens_signature(
    set: &TrackSet,
    frame: i64,
    first: i64,
    last: i64,
    settings: &ZoomSettings,
) -> bool {
    for w in 0..=settings.signature_window.max(0) {
        let a = (frame - w).max(first);
        let b = (frame + 1 + w).min(last);
        let pts = set.correspondences(a, b);
        let Some(fit) = scale_only_fit(&pts) else {
            continue;
        };
        if fit.spread < settings.signature_px {
            continue;
        }
        return fit.residual <= settings.parallax_fraction * fit.spread;
    }
    false
}

/// Whether the pair `frame → frame + 1` is explained by a scale about one
/// centre and nothing else, and whether that fit's scale agrees with what the
/// affine matrices said.
fn scale_only(set: &TrackSet, frame: i64, log_scale: f64, settings: &ZoomSettings) -> bool {
    let pts = set.correspondences(frame, frame + 1);
    let Some(fit) = scale_only_fit(&pts) else {
        return false;
    };
    fit.residual <= settings.scale_only_px
        && (fit.log_scale - log_scale).abs() <= settings.cross_check
}

/// What [`scale_only_fit`] measured.
struct ScaleFit {
    /// `ln s` of the fitted uniform scale.
    log_scale: f64,
    /// Median residual, in source pixels — what the scale could not explain.
    residual: f64,
    /// Median radial displacement the scale itself accounts for, in source
    /// pixels: `|s − 1|` times the points' median distance from their centroid.
    /// The yardstick the residual is a fraction of.
    spread: f64,
}

/// Least-squares fit of `q = s·p + t` — a uniform scale about a free centre,
/// with no rotation and no shear.
///
/// Closed form: with both point sets centred, `s` is the ratio of the
/// cross-moment to the source's second moment, and `t` follows. `None` for a
/// degenerate spread or a fit that came out mirrored, neither of which is a
/// zoom.
fn scale_only_fit(pts: &[Correspondence]) -> Option<ScaleFit> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let (mut px, mut py, mut qx, mut qy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for c in pts {
        px += c.from[0];
        py += c.from[1];
        qx += c.to[0];
        qy += c.to[1];
    }
    px /= n;
    py /= n;
    qx /= n;
    qy /= n;
    let (mut cross, mut second) = (0.0f64, 0.0f64);
    for c in pts {
        let (ax, ay) = (c.from[0] - px, c.from[1] - py);
        let (bx, by) = (c.to[0] - qx, c.to[1] - qy);
        cross += ax * bx + ay * by;
        second += ax * ax + ay * ay;
    }
    if second < 1e-9 || cross <= 0.0 {
        return None;
    }
    let s = cross / second;
    if !s.is_finite() || s < 1e-6 {
        return None;
    }
    let (tx, ty) = (qx - s * px, qy - s * py);
    let mut residuals: Vec<f64> = pts
        .iter()
        .map(|c| (s * c.from[0] + tx - c.to[0]).hypot(s * c.from[1] + ty - c.to[1]))
        .collect();
    let mut radii: Vec<f64> = pts
        .iter()
        .map(|c| (c.from[0] - px).hypot(c.from[1] - py))
        .collect();
    Some(ScaleFit {
        log_scale: s.ln(),
        residual: geom::median(&mut residuals)?,
        spread: (s - 1.0).abs() * geom::median(&mut radii)?,
    })
}

/// Set one track's state to [`TrackState::Moving`]. Lives here rather than on
/// the public store because segmentation is the only thing entitled to decide
/// it (docs/impl/tracking.md §2's "phase 2 sets it").
impl TrackSet {
    pub(crate) fn mark_moving(&mut self, id: u32) -> bool {
        let Ok(i) = self.tracks.binary_search_by_key(&id, |t| t.id) else {
            return false;
        };
        match self.tracks.get_mut(i) {
            Some(t) => {
                t.state = TrackState::Moving;
                true
            }
            None => false,
        }
    }
}
