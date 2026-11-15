//! Following a grade window: a box that moves, grows and turns with what it
//! sits on.
//!
//! # In plain terms
//!
//! A colourist draws a window round a face and wants it to stay on the face.
//! The specks inside the window are followed from frame to frame, and the move
//! that best explains all of them at once is read as a slide, a growth and a
//! turn: a similarity. Specks that disagree (one on the background showing
//! through the window's corner, one on a passing hand) are set aside by the
//! same robust search the planar track uses. The window's centre, size and
//! angle are pushed through that move.
//!
//! Each frame is measured against an anchor frame rather than the frame before,
//! for the planar track's reason: errors then add up once per re-anchor, not
//! once per frame. When too few of the anchor's specks are left, the frame just
//! done becomes the anchor. When even that cannot be measured, the window is
//! lost, and the run says so rather than guessing.
//!
//! The tracker is only allowed to find and keep specks inside the window, as
//! it is expected to be on the next frame, pulled in by a small margin so a
//! speck on the window's edge is not half background.

use crate::geom::{project, transfer_distance, Mat3};
use crate::pairs::{lo_ransac, mix, seed_for, Budget};
use crate::{
    exclude::ExclusionMask, Correspondence, FramePlane, TrackError, TrackSettings, Tracker,
};

/// A window's box in raster pixels: its centre, its full width and height, and
/// its turn in degrees, clockwise on the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowBox {
    pub centre: [f64; 2],
    pub size: [f64; 2],
    pub angle: f64,
}

/// The region the tracker is kept to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOutline {
    /// The box itself.
    Box,
    /// The ellipse inside the box.
    Ellipse,
}

/// What a window follow takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSettings {
    /// How far in from the window's edge a speck must sit, as a share of each
    /// half side.
    pub margin: f64,
    /// Agreeing specks a frame's move must stand on. Below this the window is
    /// lost.
    pub min_inliers: usize,
    /// Below this many agreeing specks against the anchor, the frame before
    /// becomes the anchor.
    pub reanchor_below: usize,
    /// The robust fit's inlier threshold, raster pixels.
    pub threshold_px: f64,
}

impl Default for WindowSettings {
    fn default() -> Self {
        WindowSettings {
            margin: 0.1,
            min_inliers: 6,
            reanchor_below: 12,
            threshold_px: 1.5,
        }
    }
}

/// Follows one window through frames pushed one at a time.
pub struct WindowFollower {
    tracker: Tracker,
    settings: WindowSettings,
    outline: WindowOutline,
    /// The step about to be pushed. Steps are the tracker's frame numbers, so
    /// a run backwards through a shot counts up like any other.
    step: i64,
    /// The frame everything is measured against, and the box on it.
    anchor: (i64, WindowBox),
    /// The last two boxes, newest last, for the guess at where the window is
    /// on the next frame.
    recent: [Option<(i64, WindowBox)>; 2],
    lost: bool,
}

impl WindowFollower {
    /// Start following `start` on a raster of `size` pixels.
    #[must_use]
    pub fn new(
        start: WindowBox,
        outline: WindowOutline,
        size: (usize, usize),
        settings: WindowSettings,
    ) -> Self {
        // Buckets small enough that the window holds a few dozen specks
        // whatever its size, and large enough that a small one is not all
        // bucket bookkeeping.
        let side = start.size[0].abs().min(start.size[1].abs());
        let cell = (side / 6.0).clamp(8.0, 64.0);
        let across = |n: usize| ((n as f64 / cell).ceil() as usize).max(1);
        let tracker = Tracker::new(TrackSettings {
            grid: (across(size.0), across(size.1)),
            // Refill a bucket as soon as it is short, so a window that thins
            // out is topped up from the very next frame.
            redetect_below: 2,
            ..TrackSettings::default()
        });
        WindowFollower {
            tracker,
            settings,
            outline,
            step: 0,
            anchor: (0, start),
            recent: [None, None],
            lost: false,
        }
    }

    /// Follow the window onto `plane`, the next frame in the order being
    /// tracked. The first push is the start frame and answers the start box.
    /// `None` once the window is lost; every push after that answers `None`
    /// too.
    ///
    /// # Errors
    ///
    /// [`TrackError`] when the plane is not the size the run started at.
    pub fn push(&mut self, plane: FramePlane<'_>) -> Result<Option<WindowBox>, TrackError> {
        if self.lost {
            return Ok(None);
        }
        let step = self.step;
        let guess = self.guess();
        self.tracker.set_masks(vec![self.region(guess)]);
        self.tracker.push(step, plane, None)?;
        self.step += 1;
        if step == 0 {
            let start = self.anchor.1;
            self.recent = [None, Some((0, start))];
            return Ok(Some(start));
        }

        let s = &self.settings;
        let (anchor, anchor_box) = self.anchor;
        let mut fit = self.fit(anchor, step);
        if fit.as_ref().is_none_or(|f| f.1 < s.reanchor_below) {
            if let Some((prev, prev_box)) = self.recent[1].filter(|p| p.0 != anchor) {
                let retry = self.fit(prev, step);
                if retry
                    .as_ref()
                    .is_some_and(|r| r.1 >= s.min_inliers && r.1 > fit.as_ref().map_or(0, |f| f.1))
                {
                    self.anchor = (prev, prev_box);
                    fit = retry;
                }
            }
        }
        let Some((m, _)) = fit.filter(|f| f.1 >= self.settings.min_inliers) else {
            self.lost = true;
            return Ok(None);
        };
        let previous = self.recent[1].map_or(anchor_box.angle, |p| p.1.angle);
        let Some(now) = moved(&m, &self.anchor.1, previous) else {
            self.lost = true;
            return Ok(None);
        };
        self.recent = [self.recent[1], Some((step, now))];
        Ok(Some(now))
    }

    /// Where the window probably is on the next frame: the last box carried on
    /// by the last frame's move.
    fn guess(&self) -> WindowBox {
        match self.recent {
            [Some((_, a)), Some((_, b))] => {
                let grow = |i: usize| {
                    if a.size[i].abs() > f64::EPSILON {
                        b.size[i] * (b.size[i] / a.size[i])
                    } else {
                        b.size[i]
                    }
                };
                WindowBox {
                    centre: [
                        2.0 * b.centre[0] - a.centre[0],
                        2.0 * b.centre[1] - a.centre[1],
                    ],
                    size: [grow(0), grow(1)],
                    angle: 2.0 * b.angle - a.angle,
                }
            }
            [_, Some((_, b))] => b,
            _ => self.anchor.1,
        }
    }

    /// The window pulled in by the margin, as the one region the tracker may
    /// work inside.
    fn region(&self, w: WindowBox) -> ExclusionMask {
        let k = (1.0 - self.settings.margin).clamp(0.05, 1.0);
        let (hw, hh) = (0.5 * w.size[0].abs() * k, 0.5 * w.size[1].abs() * k);
        let (s, c) = w.angle.to_radians().sin_cos();
        let at = |u: f64, v: f64| [w.centre[0] + u * c - v * s, w.centre[1] + u * s + v * c];
        let points = match self.outline {
            WindowOutline::Box => vec![at(-hw, -hh), at(hw, -hh), at(hw, hh), at(-hw, hh)],
            WindowOutline::Ellipse => (0..24)
                .map(|i| {
                    let t = f64::from(i) * std::f64::consts::TAU / 24.0;
                    at(hw * t.cos(), hh * t.sin())
                })
                .collect(),
        };
        ExclusionMask::from_points(points, true)
    }

    /// The similarity taking frame `from` to frame `to`, and how many specks
    /// agreed with it.
    fn fit(&self, from: i64, to: i64) -> Option<(Mat3, usize)> {
        let pts = correspondences(&self.tracker, from, to);
        if pts.len() < 2 {
            return None;
        }
        let fit = lo_ransac(
            &pts,
            &Budget {
                thr: self.settings.threshold_px,
                max_iterations: 200,
                confidence: 0.999,
                seed: mix(seed_for(from, to, pts.len()) ^ 0x57A1_D0C5_11E5_0B0F),
                sample: 2,
            },
            |sample, out| {
                out.clear();
                out.extend(similarity(sample));
            },
            similarity,
            |m, c| transfer_distance(m, c.from, c.to),
            &mut Vec::with_capacity(1),
        )?;
        Some((fit.model, fit.inliers.len()))
    }
}

/// Every track the tracker holds on both frames, in id order.
fn correspondences(tracker: &Tracker, from: i64, to: i64) -> Vec<Correspondence> {
    tracker
        .tracks
        .iter()
        .filter_map(|t| {
            let (a, b) = (t.point_at(from)?, t.point_at(to)?);
            Some(Correspondence {
                id: t.id,
                from: [a.x, a.y],
                to: [b.x, b.y],
            })
        })
        .collect()
}

/// The least-squares similarity taking every `from` to its `to`.
///
/// Written as complex numbers about the two centroids, the best `w` in
/// `z ↦ b̄ + w·(z − ā)` is `Σ conj(a)·b / Σ|a|²`: its argument is the turn and
/// its modulus the growth. `None` when the `from` points are all one point.
fn similarity(pts: &[Correspondence]) -> Option<Mat3> {
    let n = pts.len() as f64;
    if pts.len() < 2 {
        return None;
    }
    let (mut a, mut b) = ([0.0f64; 2], [0.0f64; 2]);
    for c in pts {
        a = [a[0] + c.from[0], a[1] + c.from[1]];
        b = [b[0] + c.to[0], b[1] + c.to[1]];
    }
    let (a, b) = ([a[0] / n, a[1] / n], [b[0] / n, b[1] / n]);
    let (mut re, mut im, mut d) = (0.0f64, 0.0f64, 0.0f64);
    for c in pts {
        let (ax, ay) = (c.from[0] - a[0], c.from[1] - a[1]);
        let (bx, by) = (c.to[0] - b[0], c.to[1] - b[1]);
        re += ax * bx + ay * by;
        im += ax * by - ay * bx;
        d += ax * ax + ay * ay;
    }
    if !d.is_finite() || d <= f64::EPSILON {
        return None;
    }
    let (wr, wi) = (re / d, im / d);
    if !wr.is_finite() || !wi.is_finite() || wr.hypot(wi) <= f64::EPSILON {
        return None;
    }
    Some([
        [wr, -wi, b[0] - wr * a[0] + wi * a[1]],
        [wi, wr, b[1] - wi * a[0] - wr * a[1]],
        [0.0, 0.0, 1.0],
    ])
}

/// `w` pushed through the similarity `m`, its angle kept within half a turn of
/// `near` so a spin keeps counting rather than snapping back.
fn moved(m: &Mat3, w: &WindowBox, near: f64) -> Option<WindowBox> {
    let centre = project(m, w.centre)?;
    let grow = m[0][0].hypot(m[1][0]);
    let turn = m[1][0].atan2(m[0][0]).to_degrees();
    let raw = w.angle + turn;
    let angle = raw + ((near - raw) / 360.0).round() * 360.0;
    let out = WindowBox {
        centre,
        size: [w.size[0] * grow, w.size[1] * grow],
        angle,
    };
    let finite = out.centre.iter().chain(&out.size).all(|v| v.is_finite());
    (finite && angle.is_finite()).then_some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A smooth random texture: hashed values on an 8 px lattice, blended.
    fn texture(u: f64, v: f64) -> f32 {
        let hash = |i: i64, j: i64| -> f64 {
            let h = mix((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (j as u64));
            (h >> 11) as f64 / (1u64 << 53) as f64
        };
        let (x, y) = (u / 8.0, v / 8.0);
        let (i, j) = (x.floor(), y.floor());
        let (fx, fy) = (x - i, y - j);
        let (i, j) = (i as i64, j as i64);
        let top = hash(i, j) * (1.0 - fx) + hash(i + 1, j) * fx;
        let bottom = hash(i, j + 1) * (1.0 - fx) + hash(i + 1, j + 1) * fx;
        (top * (1.0 - fy) + bottom * fy) as f32
    }

    /// A 120 px textured square on grey, at `w`.
    fn frame(w: &WindowBox, size: (usize, usize)) -> Vec<f32> {
        let grow = w.size[0] / 120.0;
        let (s, c) = w.angle.to_radians().sin_cos();
        let mut out = vec![0.5f32; size.0 * size.1];
        for y in 0..size.1 {
            for x in 0..size.0 {
                let (dx, dy) = (x as f64 + 0.5 - w.centre[0], y as f64 + 0.5 - w.centre[1]);
                // Back into the square's own pixels: undo the turn, then the growth.
                let (u, v) = ((c * dx + s * dy) / grow, (c * dy - s * dx) / grow);
                if u.abs() < 60.0 && v.abs() < 60.0 {
                    out[y * size.0 + x] = 0.2 + 0.6 * texture(u + 60.0, v + 60.0);
                }
            }
        }
        out
    }

    /// The square slides, grows by a fifth and turns fifteen degrees over
    /// thirty frames, and the window tracked forwards from its first frame
    /// stays within a pixel and a degree of it all the way.
    #[test]
    fn a_window_follows_a_square_that_slides_grows_and_turns() {
        let size = (400, 300);
        let truth = |k: usize| {
            let t = k as f64 / 29.0;
            let grow = 1.0 + 0.2 * t;
            WindowBox {
                centre: [150.0 + 90.0 * t, 140.0 + 30.0 * t],
                size: [120.0 * grow, 120.0 * grow],
                angle: 15.0 * t,
            }
        };
        let mut follower = WindowFollower::new(
            truth(0),
            WindowOutline::Box,
            size,
            WindowSettings::default(),
        );
        for k in 0..30 {
            let luma = frame(&truth(k), size);
            let got = follower
                .push(FramePlane::new(&luma, size.0, size.1).unwrap())
                .unwrap()
                .unwrap_or_else(|| panic!("lost on frame {k}"));
            let want = truth(k);
            for i in 0..2 {
                assert!(
                    (got.centre[i] - want.centre[i]).abs() < 1.0,
                    "frame {k}: centre {got:?} against {want:?}"
                );
                assert!(
                    (got.size[i] - want.size[i]).abs() < 1.0,
                    "frame {k}: size {got:?} against {want:?}"
                );
            }
            assert!(
                (got.angle - want.angle).abs() < 1.0,
                "frame {k}: angle {got:?} against {want:?}"
            );
        }
    }
}
