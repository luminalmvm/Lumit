//! What a track analysis is asked for.

use super::*;

impl AnalysisSettings {
    /// The lens the Focal length row gives, in millimetres. `None` to solve it.
    #[must_use]
    pub fn focal(&self) -> Option<f64> {
        Some(f64::from_bits(self.focal_bits)).filter(|f| f.is_finite() && *f > 0.0)
    }

    /// Read the settings off one Camera track instance. A parameter the
    /// instance does not carry — an older project, a hand-edited file — reads
    /// as the effect's own default rather than failing (docs/14 §4).
    #[must_use]
    pub fn of(fx: &EffectInstance) -> Self {
        let d = Self::default();
        AnalysisSettings {
            density: match fx.param("density") {
                Some(EffectValue::Choice(v)) => *v,
                _ => d.density,
            },
            use_masks: match fx.param("use_masks") {
                Some(EffectValue::Bool(v)) => *v,
                _ => d.use_masks,
            },
            follow: match fx.param(lumit_core::track::FOLLOW_PARAM) {
                Some(EffectValue::Choice(v)) => *v,
                _ => d.follow,
            },
            focal_bits: match fx.param("focal_length") {
                Some(EffectValue::Float(p)) => p.value_at(0.0).to_bits(),
                _ => d.focal_bits,
            },
        }
    }

    /// Whether this is one of the point options rather than the surface.
    pub(super) fn points(self) -> bool {
        self.follow != lumit_core::fx::effects::planar_track::FOLLOW_DEFAULT
    }

    pub(super) fn tracker(self) -> TrackSettings {
        let (across, down, per_bucket) =
            lumit_core::fx::effects::camera_track::density(self.density);
        TrackSettings {
            grid: (across, down),
            // A point track is confined to one or two small boxes, and the
            // detection grid is over the whole frame: a box the size of a badge
            // lands inside a bucket or two, so the ordinary two-per-bucket would
            // leave a point standing on two features. Everything outside the
            // boxes is excluded, so raising the count costs nothing anywhere
            // else — the buckets the boxes do not touch have nothing to detect.
            //
            // ponytail: one number rather than a grid sized to the region; a
            // per-region detector is the upgrade if a very small box starves.
            per_bucket: if self.points() {
                per_bucket.max(32)
            } else {
                per_bucket
            },
            ..TrackSettings::default()
        }
    }
}
