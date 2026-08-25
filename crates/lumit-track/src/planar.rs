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
