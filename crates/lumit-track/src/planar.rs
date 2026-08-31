//! Planar tracking (docs/impl/tracking.md §6, K-579): a flat patch of a shot
//! followed frame by frame, as four corners.
//!
//! # In plain terms
//!
//! Somebody in the shot is holding a phone, and you want your own picture on
//! its screen. The phone is *flat*, and a flat thing filmed by a camera has a
//! very convenient property: however the camera moves and however the phone
//! turns, what it does to the picture of that flat surface is always the same
//! kind of warp — a **homography**, the four-corner stretch a Corner pin
//! already applies. Eight numbers describe it completely.
//!
//! So the job is not "where did the phone go" but "which eight numbers, this
//! frame". The recipe is:
//!
//! 1. The user draws a quad round the flat thing on one frame — the
//!    **reference frame**. Everything else is measured against that one.
//! 2. The ordinary tracker ([`crate::Tracker`]) follows specks, told to work
//!    only inside the quad (an *inverted* exclusion mask, which is what a mask
//!    the tracker must stay within already means).
//! 3. For every later frame, take every speck that is on both the reference
//!    frame and this one, and find the warp that best explains all of them at
//!    once. Specks that disagree with the majority — one that crawled off onto
//!    the hand holding the phone, one that latched onto a reflection — are set
//!    aside by the same robust search the camera solve uses.
//! 4. Push the four corners through that warp. Those are the answer.
//!
//! # Why it is measured from the reference frame and not from the frame before
//!
//! The obvious way is to warp frame 1 onto frame 2, frame 2 onto frame 3, and
//! multiply as you go. It is also the way that **drifts**: every step's small
//! error is multiplied into every step after it, and by frame three hundred
//! the quad has quietly walked off the phone. Measuring each frame against the
//! *reference* frame instead makes every frame's error independent — frame
//! three hundred is no worse than frame two, because nothing about frame two
//! went into it.
//!
//! The price is that the reference frame's specks die out: they leave the
//! picture, the phone turns away, someone walks in front. When too few of them
//! are left to trust, the tracker **re-anchors** — it starts measuring against
//! a recent frame instead, and remembers the one warp that takes the reference
//! frame to that anchor. Error then accumulates once per re-anchor rather than
//! once per frame, which over a long shot is the difference between a handful
//! of small errors and thousands.
//!
//! # Refusing rather than guessing
//!
//! A quad drawn over a blank wall has nothing in it to follow, and no amount of
//! arithmetic invents it. The answer is [`PlanarError::TooFewFeatures`] — a
//! refusal, calmly, the way every other refusal in this crate is one. A track
//! that starts well and then loses the surface part-way stops there and hands
//! back the span that worked, exactly as the camera analysis truncates at a
//! severed frame (docs/impl/tracking.md §5d).

use crate::geom::{self, project};
use crate::pairs::homography_ransac;
use crate::{GeometrySettings, Mat3, TrackSet};

/// The four corners of the tracked quad, in **source raster pixels**, in the
/// order Corner pin declares them: upper left, upper right, lower left, lower
/// right (docs/08 §3.48).
///
/// An array rather than a struct with four named fields because every operation
/// on it is the same operation four times, and naming them would mean writing
/// each of those four times out.
pub type Quad = [[f64; 2]; 4];

/// What a planar track was asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanarSettings {
    /// The robust fit's budget and inlier threshold — the same knobs the
    /// two-view geometry uses, because it is the same LO-RANSAC underneath.
    pub geometry: GeometrySettings,
    /// How many agreeing correspondences a frame's homography must stand on.
    /// Four is the minimal sample, so a fit through exactly four has nothing
    /// left over to disagree with it; **six** is the smallest set that is
    /// verified rather than merely fitted, with two observations spare.
    pub min_inliers: usize,
    /// Below this many agreeing correspondences against the current anchor, the
    /// tracker re-anchors to the previous frame rather than carrying on with a
    /// thinning set. Above `min_inliers` deliberately: re-anchoring costs one
    /// composition's worth of error, and doing it while the fit is still sound
    /// is cheaper than doing it after the fit has gone soft.
    pub reanchor_below: usize,
}

impl Default for PlanarSettings {
    fn default() -> Self {
        PlanarSettings {
            geometry: GeometrySettings::default(),
            min_inliers: 6,
            reanchor_below: 12,
        }
    }
}

/// One frame's answer.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanarFrame {
    /// The source frame number this describes.
    pub frame: i64,
    /// The quad's corners on this frame, in source raster pixels.
    pub corners: Quad,
    /// How many correspondences agreed with the warp that produced them. One
    /// number per frame is what makes a soft stretch of a track visible without
    /// re-deriving anything.
    pub inliers: u32,
    /// Whether this frame was measured against the reference frame directly
    /// (`false`) or against a later anchor (`true`). A run with no re-anchor at
    /// all carries no accumulated error whatsoever, and that is worth being able
    /// to see.
    pub reanchored: bool,
}

/// A whole planar track: the reference frame's quad, and where it lands on
/// every frame that could be measured.
///
/// The span is a prefix of the clip by construction — the run follows the
/// source from its first frame and can only ever stop early — so `frames`'
/// first and last entries are the whole of its extent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanarTrack {
    /// The frame the quad was drawn on, and everything is measured against.
    pub reference_frame: i64,
    /// The quad as drawn, in source raster pixels.
    pub reference_quad: Quad,
    /// One entry per followed frame, ascending. Never empty: a track with
    /// nothing in it is [`PlanarError::TooFewFeatures`] instead.
    pub frames: Vec<PlanarFrame>,
    /// How many times the measurement was re-anchored over the run. Zero is a
    /// track with no accumulated drift at all.
    pub reanchors: u32,
}

impl PlanarTrack {
    /// The corners on `frame`, or `None` outside the followed span.
    #[must_use]
    pub fn corners_at(&self, frame: i64) -> Option<Quad> {
        // Frames are ascending and contiguous from the first followed one, so
        // this is an index rather than a search — but the search is written out
        // because a re-anchor is free to skip a frame it could not fit, and an
        // index would then answer about the wrong one.
        self.frames
            .binary_search_by_key(&frame, |f| f.frame)
            .ok()
            .and_then(|i| self.frames.get(i))
            .map(|f| f.corners)
    }

    /// The first and last frames the track covers.
    #[must_use]
    pub fn frame_range(&self) -> Option<(i64, i64)> {
        Some((self.frames.first()?.frame, self.frames.last()?.frame))
    }
}

/// Why a planar track produced nothing. Every variant is a refusal rather than
/// a fault — the pictures inside the quad did not carry the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanarError {
    /// The tracker followed nothing at all inside the quad — a blank wall, a
    /// patch of sky, a quad drawn off the picture.
    #[error("too little inside the quad could be followed")]
    TooFewFeatures,
    /// Features were followed, but no frame's correspondences ever settled on a
    /// warp: the patch is not flat, or what is inside the quad moves against
    /// itself.
    #[error("the quad's contents are not a single flat surface")]
    NotPlanar,
    /// The caller stopped it.
    #[error("the track was cancelled")]
    Cancelled,
}

/// The identity homography — what takes the reference frame to itself.
const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Follow `quad` — drawn on `reference_frame` in source raster pixels — through
/// every frame of `set`.
///
/// `source_size` is the raster the tracks live in; it is what turns
/// [`GeometrySettings::pixel_threshold`] into the units the fit is measured in
/// (the conditioning [`homography_ransac`] applies).
#[must_use = "a planar track that is thrown away cost a whole analysis"]
pub fn solve_planar(
    set: &TrackSet,
    reference_frame: i64,
    quad: Quad,
    source_size: (usize, usize),
    settings: &PlanarSettings,
) -> Result<PlanarTrack, PlanarError> {
    solve_planar_cancellable(set, reference_frame, quad, source_size, settings, &|| false)
}

/// [`solve_planar`], asked between frames whether to stop.
///
/// The frame loop is the cancellation seam, as it is everywhere else in this
/// crate (docs/14 §1.4): one check per frame, and a stopped run returns
/// [`PlanarError::Cancelled`] with **nothing partial** rather than a half-filled
/// track that looks finished.
pub fn solve_planar_cancellable(
    set: &TrackSet,
    reference_frame: i64,
    quad: Quad,
    source_size: (usize, usize),
    settings: &PlanarSettings,
    stop: &dyn Fn() -> bool,
) -> Result<PlanarTrack, PlanarError> {
    let (first, last) = set.frame_range().ok_or(PlanarError::TooFewFeatures)?;
    let reference = reference_frame.clamp(first, last);

    // The anchor the current frame is measured against, and the warp that takes
    // the reference frame to it. Both start at the reference frame itself,
    // where the warp is the identity by definition.
    let mut anchor = reference;
    let mut to_anchor = IDENTITY;
    let mut anchors = 0u32;
    // The reference-to-frame warp of the frame just done, which is what a
    // re-anchor adopts: it is the composition already paid for, so re-anchoring
    // adds no error of its own beyond what that frame already carried.
    let mut previous: Option<(i64, Mat3)> = None;

    let mut frames: Vec<PlanarFrame> = Vec::with_capacity(
        usize::try_from(last.saturating_sub(first).saturating_add(1)).unwrap_or(0),
    );
    frames.push(PlanarFrame {
        frame: reference,
        corners: quad,
        // Every correspondence agrees with the identity, but claiming a count
        // here would be claiming a measurement that was never made. The
        // reference frame is where the quad *is*, by definition.
        inliers: 0,
        reanchored: false,
    });

    for frame in (reference + 1)..=last {
        if stop() {
            return Err(PlanarError::Cancelled);
        }
        let mut fit = fit_frame(set, anchor, frame, source_size, settings);

        // Thinning against the current anchor: adopt the previous frame as the
        // new one and try again. Only ever once per frame — if the fresh anchor
        // cannot explain the very next frame either, the surface has gone and
        // no third attempt will find it.
        let thin = fit.as_ref().is_none_or(|f| f.1 < settings.reanchor_below);
        if thin {
            if let Some((prev_frame, prev_h)) = previous {
                if prev_frame != anchor {
                    let retry = fit_frame(set, prev_frame, frame, source_size, settings);
                    if retry.as_ref().is_some_and(|r| {
                        r.1 >= settings.min_inliers && r.1 > fit.as_ref().map_or(0, |f| f.1)
                    }) {
                        anchor = prev_frame;
                        to_anchor = prev_h;
                        anchors += 1;
                        fit = retry;
                    }
                }
            }
        }

        let Some((h, inliers)) = fit.filter(|f| f.1 >= settings.min_inliers) else {
            // The surface stopped being followable. The span that worked is a
            // real answer about a real stretch of shot; the frames after it are
            // not a poorer answer, they are no answer.
            break;
        };
        let to_frame = geom::mul3(&h, &to_anchor);
        let Some(corners) = warp_quad(&to_frame, quad) else {
            break;
        };
        frames.push(PlanarFrame {
            frame,
            corners,
            inliers: u32::try_from(inliers).unwrap_or(u32::MAX),
            reanchored: anchor != reference,
        });
        previous = Some((frame, to_frame));
    }

    if frames.len() < 2 {
        // Nothing but the reference frame itself. Which of the two refusals it
        // is depends on whether there was anything to follow at all: no
        // correspondences is a starved patch, correspondences that never agreed
        // is a surface that is not one.
        return Err(
            if set.correspondences(reference, reference + 1).is_empty() {
                PlanarError::TooFewFeatures
            } else {
                PlanarError::NotPlanar
            },
        );
    }

    Ok(PlanarTrack {
        reference_frame: reference,
        reference_quad: quad,
        frames,
        reanchors: anchors,
    })
}

/// The homography from `from` to `to` over the tracks present on both, and how
/// many of them agreed with it.
fn fit_frame(
    set: &TrackSet,
    from: i64,
    to: i64,
    source_size: (usize, usize),
    settings: &PlanarSettings,
) -> Option<(Mat3, usize)> {
    let pts = set.correspondences(from, to);
    let (h, inliers) = homography_ransac(&pts, source_size, from, to, &settings.geometry)?;
    Some((h, inliers.len()))
}

/// Push all four corners through `h`. `None` if any of them maps to infinity —
/// a quad with one corner on the horizon is not a quad, and half a warped one
/// would be a silent lie.
fn warp_quad(h: &Mat3, quad: Quad) -> Option<Quad> {
    let mut out = [[0.0f64; 2]; 4];
    for (slot, corner) in out.iter_mut().zip(quad) {
        let p = project(h, corner)?;
        if !p[0].is_finite() || !p[1].is_finite() {
            return None;
        }
        *slot = p;
    }
    Some(out)
}

/// The quad as an outline the tracker can be told to stay inside — the polygon
/// in drawing order (upper left, upper right, lower right, lower left), which is
/// not the order [`Quad`] stores them in.
///
/// [`Quad`] is in Corner pin's declaration order, and walking a polygon in that
/// order draws a bow tie: a self-crossing outline whose even-odd test excludes
/// the middle of the quad and includes two triangles outside it. One reordering,
/// here, rather than a second convention for callers to remember.
#[must_use]
pub fn quad_outline(quad: Quad) -> Vec<[f64; 2]> {
    vec![quad[0], quad[1], quad[3], quad[2]]
}

// ---------------------------------------------------------------------------
// Point tracking (K-735)
// ---------------------------------------------------------------------------

/// What a point track was asked for.
///
/// # In plain terms
///
/// A planar track asks "which eight numbers is this frame", and it can only ask
/// that of something flat. Sometimes there is nothing flat to ask about — a
/// light on a car, a badge on a moving shoulder, two marks on opposite sides of
/// a room. Then the honest question is much smaller: **where did this speck
/// go**, asked of one small patch at a time.
///
/// Each patch is followed on its own, so two of them need no relation to each
/// other whatever — different depths, different objects, opposite corners of
/// the shot. One patch gives a position. Two give a position, a turn and a
/// growth, from the line between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointSettings {
    /// How far from a point a feature may sit and still be counted as part of
    /// it, in source raster pixels — half the search box's side.
    pub radius: f64,
    /// How many agreeing features a point's move must stand on. Three, so the
    /// median that decides it has a middle rather than an average of two.
    pub min_inliers: usize,
    /// Below this many, the run re-anchors to the previous frame rather than
    /// carrying on against a thinning set — [`PlanarSettings::reanchor_below`]'s
    /// reasoning at a point track's smaller scale.
    pub reanchor_below: usize,
}

impl Default for PointSettings {
    fn default() -> Self {
        PointSettings {
            radius: 40.0,
            min_inliers: 3,
            reanchor_below: 6,
        }
    }
}

/// Follow one or two **points** — not a surface — through every frame of `set`,
/// and report the answer as the same [`PlanarTrack`] a planar run produces.
///
/// `points` are on `reference_frame`, in source raster pixels; `quad` is the
/// shape the answer is reported as, warped by whatever the points did. One point
/// can only say where it went, so the warp is a **translation**; two can also
/// say how the line between them turned and stretched, so it is a
/// **similarity**. Neither is ever a perspective warp, and that is the point:
/// the two patches are not assumed to lie on one plane, or on one object.
///
/// The answer being a `PlanarTrack` is deliberate. Downstream — the store, the
/// sidecar, the status row, the Corner pin, the transform keys — a track is a
/// quad per frame, and a point track that invented a second answer shape would
/// make every one of those a union to unwrap before it could be read.
#[must_use = "a point track that is thrown away cost a whole analysis"]
pub fn solve_points(
    set: &TrackSet,
    reference_frame: i64,
    points: &[[f64; 2]],
    quad: Quad,
    settings: &PointSettings,
) -> Result<PlanarTrack, PlanarError> {
    solve_points_cancellable(set, reference_frame, points, quad, settings, &|| false)
}

/// [`solve_points`], asked between frames whether to stop — the crate's one
/// cancellation shape (docs/14 §1.4), and a stopped run returns nothing partial.
pub fn solve_points_cancellable(
    set: &TrackSet,
    reference_frame: i64,
    points: &[[f64; 2]],
    quad: Quad,
    settings: &PointSettings,
    stop: &dyn Fn() -> bool,
) -> Result<PlanarTrack, PlanarError> {
    if points.is_empty() || points.len() > 2 {
        return Err(PlanarError::TooFewFeatures);
    }
    let (first, last) = set.frame_range().ok_or(PlanarError::TooFewFeatures)?;
    let reference = reference_frame.clamp(first, last);

    // Where each point is on the frame everything is currently measured against,
    // and where it was on the reference frame. Positions are absolute, so a
    // re-anchor composes nothing — it simply starts measuring from a nearer
    // frame, which is the whole of the drift argument met a second time.
    let mut anchor = reference;
    let mut at_anchor: Vec<[f64; 2]> = points.to_vec();
    let mut anchors = 0u32;
    let mut previous: Option<(i64, Vec<[f64; 2]>)> = None;

    let mut frames: Vec<PlanarFrame> = Vec::with_capacity(
        usize::try_from(last.saturating_sub(first).saturating_add(1)).unwrap_or(0),
    );
    frames.push(PlanarFrame {
        frame: reference,
        corners: quad,
        // The reference frame is where the points *are*, by definition; a count
        // here would be claiming a measurement nobody made.
        inliers: 0,
        reanchored: false,
    });

    for frame in (reference + 1)..=last {
        if stop() {
            return Err(PlanarError::Cancelled);
        }
        let mut moved = follow(set, anchor, frame, &at_anchor, settings.radius);

        // Thinning against the current anchor: adopt the previous frame and try
        // again, once, for [`solve_planar_cancellable`]'s reason.
        let thin = moved.as_ref().is_none_or(|m| m.1 < settings.reanchor_below);
        if thin {
            if let Some((prev_frame, prev_at)) = &previous {
                if *prev_frame != anchor {
                    let retry = follow(set, *prev_frame, frame, prev_at, settings.radius);
                    if retry.as_ref().is_some_and(|r| {
                        r.1 >= settings.min_inliers && r.1 > moved.as_ref().map_or(0, |m| m.1)
                    }) {
                        anchor = *prev_frame;
                        at_anchor.clone_from(prev_at);
                        anchors += 1;
                        moved = retry;
                    }
                }
            }
        }

        let Some((now, inliers)) = moved.filter(|m| m.1 >= settings.min_inliers) else {
            // The points stopped being followable. The span that worked is a
            // whole answer about a real stretch of shot (docs/impl/tracking.md
            // §5d), and the frames after it are no answer rather than a poor one.
            break;
        };
        let Some(warp) = warp_between(points, &now) else {
            break;
        };
        let Some(corners) = warp_quad(&warp, quad) else {
            break;
        };
        frames.push(PlanarFrame {
            frame,
            corners,
            inliers: u32::try_from(inliers).unwrap_or(u32::MAX),
            reanchored: anchor != reference,
        });
        previous = Some((frame, now));
    }

    if frames.len() < 2 {
        // One refusal, not the surface's two. "Not one flat surface" is a
        // statement about a plane, and a point track never asked for one: a box
        // that could not be followed had too little in it to follow, whether the
        // features were absent or merely useless.
        return Err(PlanarError::TooFewFeatures);
    }

    Ok(PlanarTrack {
        reference_frame: reference,
        reference_quad: quad,
        frames,
        reanchors: anchors,
    })
}

/// Where each point of `at_anchor` has got to on `to`, and how many features the
/// weakest of them stood on.
///
/// Each point takes the **median** step of the features that were within
/// `radius` of it on the anchor frame. A median rather than a fit: two numbers
/// is all a point has to give, and the median is already the robust estimate of
/// them — a feature that crawled onto a passing hand is outvoted, not weighted.
fn follow(
    set: &TrackSet,
    from: i64,
    to: i64,
    at_anchor: &[[f64; 2]],
    radius: f64,
) -> Option<(Vec<[f64; 2]>, usize)> {
    let pts = set.correspondences(from, to);
    if pts.is_empty() {
        return None;
    }
    let r2 = radius * radius;
    let mut out = Vec::with_capacity(at_anchor.len());
    let mut weakest = usize::MAX;
    let (mut dx, mut dy) = (Vec::new(), Vec::new());
    for centre in at_anchor {
        dx.clear();
        dy.clear();
        for c in &pts {
            let (ex, ey) = (c.from[0] - centre[0], c.from[1] - centre[1]);
            if ex * ex + ey * ey <= r2 {
                dx.push(c.to[0] - c.from[0]);
                dy.push(c.to[1] - c.from[1]);
            }
        }
        if dx.is_empty() {
            return None;
        }
        weakest = weakest.min(dx.len());
        out.push([centre[0] + median(&mut dx), centre[1] + median(&mut dy)]);
    }
    Some((out, weakest))
}

/// The middle value of `v`, which this reorders. Even counts take the mean of
/// the two middles; `total_cmp` so the order — and therefore the answer — does
/// not depend on how the compiler feels about a NaN.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// The warp taking the reference points to where they are now: a translation
/// from one pair, a similarity from two.
///
/// The similarity is one complex division. Writing the plane as complex numbers,
/// the map that takes `a₀ → b₀` and `a₁ → b₁` while keeping angles is
/// `z ↦ b₀ + w·(z − a₀)` with `w = (b₁ − b₀) / (a₁ − a₀)` — `w`'s argument is
/// the turn and its modulus the growth, which is exactly what two points can
/// say and all they can say. `None` when the two reference points are the same
/// point, where there is no line between them to turn.
fn warp_between(reference: &[[f64; 2]], now: &[[f64; 2]]) -> Option<Mat3> {
    let (r0, n0) = (*reference.first()?, *now.first()?);
    let (Some(r1), Some(n1)) = (reference.get(1), now.get(1)) else {
        return Some([
            [1.0, 0.0, n0[0] - r0[0]],
            [0.0, 1.0, n0[1] - r0[1]],
            [0.0, 0.0, 1.0],
        ]);
    };
    let (ax, ay) = (r1[0] - r0[0], r1[1] - r0[1]);
    let (bx, by) = (n1[0] - n0[0], n1[1] - n0[1]);
    let d = ax * ax + ay * ay;
    if !d.is_finite() || d <= 0.0 {
        return None;
    }
    let (wr, wi) = ((bx * ax + by * ay) / d, (by * ax - bx * ay) / d);
    if !wr.is_finite() || !wi.is_finite() {
        return None;
    }
    Some([
        [wr, -wi, n0[0] - wr * r0[0] + wi * r0[1]],
        [wi, wr, n0[1] - wi * r0[0] - wr * r0[1]],
        [0.0, 0.0, 1.0],
    ])
}

/// The axis-aligned box round `points`, grown by `radius`, in [`Quad`] order —
/// what a point track reports as its shape, and what the tracker is confined to.
#[must_use]
pub fn points_quad(points: &[[f64; 2]], radius: f64) -> Quad {
    let r = if radius.is_finite() && radius > 0.0 {
        radius
    } else {
        1.0
    };
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in points {
        x0 = x0.min(p[0] - r);
        y0 = y0.min(p[1] - r);
        x1 = x1.max(p[0] + r);
        y1 = y1.max(p[1] + r);
    }
    if points.is_empty() {
        (x0, y0, x1, y1) = (0.0, 0.0, r, r);
    }
    [[x0, y0], [x1, y0], [x0, y1], [x1, y1]]
}

/// One search box per point, each as its own closed outline in drawing order —
/// the contours a two-point track's single exclusion region is made of.
#[must_use]
pub fn point_outlines(points: &[[f64; 2]], radius: f64) -> Vec<Vec<[f64; 2]>> {
    let r = if radius.is_finite() && radius > 0.0 {
        radius
    } else {
        1.0
    };
    points
        .iter()
        .map(|p| {
            vec![
                [p[0] - r, p[1] - r],
                [p[0] + r, p[1] - r],
                [p[0] + r, p[1] + r],
                [p[0] - r, p[1] + r],
            ]
        })
        .collect()
}
