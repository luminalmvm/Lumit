//! Blob tracking: bright or dark regions followed by their centres.
//!
//! Each frame is cut at a brightness, the pixels on the kept side are grouped
//! into connected regions, and each region's centre is matched to the nearest
//! one on the frame before. A region nothing matches starts a new track.
//!
//! Deterministic for the reason the feature tracker is: regions are found in
//! scan order, the oldest track is matched first, and a tie goes to the
//! region found first.

use crate::{FramePlane, TrackError};

/// What counts as a blob, and when two of them are the same one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlobSettings {
    /// Luma, 0..1. A pixel brighter than this is inside a blob.
    pub threshold: f32,
    /// Follow dark regions on a light picture instead.
    pub invert: bool,
    /// The smallest region kept, in pixels.
    pub min_area: f64,
    /// The largest region kept, in pixels.
    pub max_area: f64,
    /// How far a blob's centre may move between two frames and still be the
    /// same blob, in pixels.
    pub max_move: f64,
    /// The most blobs kept on one frame. Past it the largest are kept.
    pub max_blobs: usize,
}

/// Where a blob was on one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlobPoint {
    /// Source frame index, as the caller numbered it.
    pub frame: i64,
    /// The centre of the region, source raster pixels.
    pub x: f64,
    pub y: f64,
    /// How many pixels the region covers.
    pub area: f64,
}

/// One followed blob: a point per frame from where it appeared to where it
/// was lost, with no gaps.
#[derive(Clone, Debug, PartialEq)]
pub struct BlobTrack {
    /// Stable within a run, given out in the order blobs appear and never
    /// reused.
    pub id: u32,
    /// Never empty.
    pub points: Vec<BlobPoint>,
}

/// The blob tracker: push frames in order, take the tracks at the end. One
/// frame per call, so the caller's loop is where a run is cancelled.
pub struct BlobTracker {
    settings: BlobSettings,
    tracks: Vec<BlobTrack>,
    /// The tracks seen on the last frame, as indices into `tracks`. Always in
    /// id order.
    live: Vec<usize>,
    /// Which pixels a region has already claimed. Kept between frames so a
    /// frame costs no allocation of its own.
    seen: Vec<bool>,
    /// The flood fill's to-do list, kept for the same reason.
    stack: Vec<usize>,
    last_frame: Option<i64>,
    size: Option<(usize, usize)>,
}

impl BlobTracker {
    #[must_use]
    pub fn new(settings: BlobSettings) -> Self {
        BlobTracker {
            settings,
            tracks: Vec::new(),
            live: Vec::new(),
            seen: Vec::new(),
            stack: Vec::new(),
            last_frame: None,
            size: None,
        }
    }

    /// Take the next frame. Frames must be pushed in increasing order and
    /// every frame must be the same size, as the feature tracker asks.
    ///
    /// # Errors
    ///
    /// [`TrackError::FrameOrder`] out of order, [`TrackError::SizeChanged`] on
    /// a different raster.
    pub fn push(&mut self, frame: i64, plane: FramePlane<'_>) -> Result<(), TrackError> {
        if let Some(last) = self.last_frame {
            if frame <= last {
                return Err(TrackError::FrameOrder { got: frame, last });
            }
            // A track has a point on every frame it lives through, so a
            // skipped frame ends them all.
            if frame != last + 1 {
                self.live.clear();
            }
        }
        match self.size {
            None => self.size = Some((plane.w, plane.h)),
            Some((sw, sh)) if sw == plane.w && sh == plane.h => {}
            Some((sw, sh)) => {
                return Err(TrackError::SizeChanged {
                    frame,
                    w: plane.w,
                    h: plane.h,
                    sw,
                    sh,
                })
            }
        }
        self.last_frame = Some(frame);

        let found = self.regions(plane);
        let reach = self.settings.max_move * self.settings.max_move;
        let mut taken = vec![false; found.len()];
        let mut live = Vec::with_capacity(found.len());
        // ponytail: every live track against every region, which is the
        // square of `max_blobs` at worst. A bucket grid over the centres is
        // the upgrade if a noisy clip makes analysis crawl.
        for &index in &self.live {
            let Some(track) = self.tracks.get_mut(index) else {
                continue;
            };
            let Some(last) = track.points.last().copied() else {
                continue;
            };
            // Strictly nearer wins, so a tie stays with the region found
            // first.
            let mut best: Option<(usize, f64)> = None;
            for (j, (blob, taken)) in found.iter().zip(&taken).enumerate() {
                let d = (blob.x - last.x).powi(2) + (blob.y - last.y).powi(2);
                if !taken && d <= reach && best.is_none_or(|(_, nearest)| d < nearest) {
                    best = Some((j, d));
                }
            }
            let Some((j, _)) = best else {
                continue;
            };
            if let (Some(blob), Some(taken)) = (found.get(j), taken.get_mut(j)) {
                *taken = true;
                track.points.push(BlobPoint { frame, ..*blob });
                live.push(index);
            }
        }
        for (blob, taken) in found.iter().zip(&taken) {
            if *taken {
                continue;
            }
            let id = u32::try_from(self.tracks.len()).unwrap_or(u32::MAX);
            live.push(self.tracks.len());
            self.tracks.push(BlobTrack {
                id,
                points: vec![BlobPoint { frame, ..*blob }],
            });
        }
        self.live = live;
        Ok(())
    }

    /// Finish and hand over the tracks, in id order.
    #[must_use]
    pub fn finish(self) -> Vec<BlobTrack> {
        self.tracks
    }

    /// The regions of one frame that are inside the area range, in scan
    /// order of their first pixel. `frame` is left at zero for the caller.
    fn regions(&mut self, plane: FramePlane<'_>) -> Vec<BlobPoint> {
        let (w, h) = (plane.w, plane.h);
        let BlobSettings {
            threshold,
            invert,
            min_area,
            max_area,
            max_blobs,
            ..
        } = self.settings;
        let inside = |i: usize| {
            plane
                .luma
                .get(i)
                .is_some_and(|v| (*v > threshold) != invert)
        };
        self.seen.clear();
        self.seen.resize(w * h, false);
        let mut found = Vec::new();
        for start in 0..w * h {
            if self.seen.get(start).copied().unwrap_or(true) || !inside(start) {
                continue;
            }
            // Flood out from the first pixel of a region nobody has claimed.
            // The sums are of whole numbers, so the order the fill walks in
            // cannot change the centre.
            let (mut area, mut sum_x, mut sum_y) = (0.0f64, 0.0f64, 0.0f64);
            self.stack.push(start);
            if let Some(s) = self.seen.get_mut(start) {
                *s = true;
            }
            while let Some(i) = self.stack.pop() {
                let (x, y) = (i % w, i / w);
                area += 1.0;
                sum_x += x as f64;
                sum_y += y as f64;
                // The eight neighbours, so a region joined only at a corner
                // is still one region.
                for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        let j = ny * w + nx;
                        if inside(j) {
                            if let Some(s) = self.seen.get_mut(j).filter(|s| !**s) {
                                *s = true;
                                self.stack.push(j);
                            }
                        }
                    }
                }
            }
            if area >= min_area && area <= max_area {
                found.push(BlobPoint {
                    frame: 0,
                    x: sum_x / area,
                    y: sum_y / area,
                    area,
                });
            }
        }
        if found.len() > max_blobs {
            // A stable sort, so equal areas stay in scan order.
            found.sort_by(|a, b| b.area.total_cmp(&a.area));
            found.truncate(max_blobs);
        }
        found
    }
}
