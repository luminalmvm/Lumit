//! Exclusion regions (docs/impl/tracking.md §2, K-408).
//!
//! In plain terms: the user draws a shape round the thing that moves by itself —
//! an actor, a car, a flag — and the tracker refuses to put features there. A
//! feature is never *born* inside one, and a feature that wanders into one is
//! ended rather than followed, because a track on a moving object tells the
//! camera solve a lie about where the camera was.
//!
//! The shape arrives as the flattened mask polyline K-408 already carries to the
//! engine, so masks drawn for effects and masks drawn for the tracker are the
//! same geometry through the same seam.

use lumit_core::mask::{flatten_path, Mask, MaskPolyline, MASK_PATH_TOLERANCE_PX};

/// One region the tracker must keep out of.
///
/// Points are in **source raster pixels**, matching the coordinates a
/// [`TrackSet`](crate::TrackSet) stores (K-248: the tracker runs on the full,
/// unaltered footage). `MaskPolyline` arrives in px@comp, so the constructors
/// take the comp→source factor and apply it once here rather than per test.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExclusionMask {
    points: Vec<[f64; 2]>,
    inverted: bool,
}

impl ExclusionMask {
    /// Build from an already-flattened polyline. `inverted` follows the mask's
    /// own flag: normally the region *inside* the outline is excluded; inverted,
    /// everything *outside* it is, so the tracker works only within the shape.
    #[must_use]
    pub fn from_polyline(poly: &MaskPolyline, inverted: bool, comp_to_source: f64) -> Self {
        let s = if comp_to_source.is_finite() && comp_to_source > 0.0 {
            comp_to_source
        } else {
            1.0
        };
        ExclusionMask {
            points: poly
                .points
                .iter()
                .map(|p| [f64::from(p[0]) * s, f64::from(p[1]) * s])
                .collect(),
            inverted,
        }
    }

    /// Build from a document mask at comp time `t`, flattening its path at the
    /// standard tolerance and taking its `inverted` flag.
    #[must_use]
    pub fn from_mask(mask: &Mask, t: f64, comp_to_source: f64) -> Self {
        let poly = flatten_path(&mask.path_at(t), MASK_PATH_TOLERANCE_PX);
        Self::from_polyline(&poly, mask.inverted, comp_to_source)
    }

    /// The outline, in source raster pixels, and whether it is inverted.
    ///
    /// Exposed because the analysis cache is keyed by what an analysis was
    /// *given* (K-417): the mask geometry decides which tracks exist, so it
    /// belongs in the key — and two masks that flatten to the same outline
    /// deserve the same cached answer, which hashing ids would not give.
    #[must_use]
    pub fn outline(&self) -> (&[[f64; 2]], bool) {
        (&self.points, self.inverted)
    }

    /// Whether `(x, y)` — in source raster pixels — is forbidden.
    ///
    /// Even-odd crossing count against the polyline. A degenerate outline
    /// (fewer than three points) excludes nothing when upright and everything
    /// when inverted, which is what those two shapes mean; either way it is an
    /// answer, never a fault (14-ENGINEERING-RULES §4).
    #[must_use]
    pub fn excludes(&self, x: f64, y: f64) -> bool {
        if self.points.len() < 3 {
            return self.inverted;
        }
        let mut inside = false;
        let n = self.points.len();
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (self.points[i], self.points[j]);
            if (a[1] > y) != (b[1] > y) {
                let dy = b[1] - a[1];
                if dy != 0.0 && x < a[0] + (y - a[1]) * (b[0] - a[0]) / dy {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside != self.inverted
    }
}

/// Whether any of `masks` forbids `(x, y)`.
pub(crate) fn excluded(masks: &[ExclusionMask], x: f64, y: f64) -> bool {
    masks.iter().any(|m| m.excludes(x, y))
}
