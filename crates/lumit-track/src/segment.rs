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
    /// `|log scale|` per frame pair above which the lens is considered to be
    /// moving at all. Below this is tracker noise.
    pub ramp_threshold: f64,
    /// `|log scale|` a single isolated pair must exceed before it can be a cut
    /// rather than a one-frame ramp. A scope-in is an enormous number here; an
    /// operator's hand is not.
    pub cut_threshold: f64,
    /// Residual, in source pixels, that a scale-only fit to the pair's
    /// displacements must stay under for the pair to be a cut. This is what
    /// separates "the lens changed" from "the camera lunged".
    pub scale_only_px: f64,
    /// How far the scale-only fit's own `log scale` may differ from the median
    /// of the tracker's affine matrices before the two are not describing the
    /// same event. The note's cross-check, made a number.
    pub cross_check: f64,
}

impl Default for ZoomSettings {
    fn default() -> Self {
        ZoomSettings {
            ramp_threshold: 0.004,
            cut_threshold: 0.05,
            scale_only_px: 1.5,
            cross_check: 0.05,
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
/// Returned in frame order.
#[must_use]
pub fn detect_zoom(set: &TrackSet, settings: &ZoomSettings) -> Vec<ZoomBoundary> {
    let mut out = Vec::new();
    let Some((first, last)) = set.frame_range() else {
        return out;
    };
    let hot = |f: i64| {
        set.median_log_scale(f)
            .filter(|v| v.abs() > settings.ramp_threshold)
    };

    let mut frame = first;
    while frame < last {
        let Some(head) = hot(frame) else {
            frame += 1;
            continue;
        };
        let mut run = vec![head];
        let mut end = frame;
        while end + 1 < last {
            let Some(v) = hot(end + 1) else {
                break;
            };
            run.push(v);
            end += 1;
        }
        let log_scale = geom::median(&mut run).unwrap_or(head);
        let kind = if end == frame
            && head.abs() > settings.cut_threshold
            && scale_only(set, frame, head, settings)
        {
            ZoomKind::Cut
        } else {
            ZoomKind::Ramp
        };
        out.push(ZoomBoundary {
            frame,
            end_frame: end,
            kind,
            log_scale,
        });
        frame = end + 1;
    }
    out
}

/// Whether the pair `frame → frame + 1` is explained by a scale about one
/// centre and nothing else, and whether that fit's scale agrees with what the
/// affine matrices said.
fn scale_only(set: &TrackSet, frame: i64, log_scale: f64, settings: &ZoomSettings) -> bool {
    let pts = set.correspondences(frame, frame + 1);
    let Some((fitted, residual)) = scale_only_fit(&pts) else {
        return false;
    };
    residual <= settings.scale_only_px && (fitted - log_scale).abs() <= settings.cross_check
}

/// Least-squares fit of `q = s·p + t` — a uniform scale about a free centre,
/// with no rotation and no shear. Returns `(log s, median residual)`.
///
/// Closed form: with both point sets centred, `s` is the ratio of the
/// cross-moment to the source's second moment, and `t` follows. `None` for a
/// degenerate spread or a fit that came out mirrored, neither of which is a
/// zoom.
fn scale_only_fit(pts: &[Correspondence]) -> Option<(f64, f64)> {
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
    Some((s.ln(), geom::median(&mut residuals)?))
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
