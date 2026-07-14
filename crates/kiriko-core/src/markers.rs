//! Timeline markers (docs/03-DATA-MODEL.md §11): user cue points, chapters, and
//! automatically-detected beat markers. Beat markers carry provenance so that
//! regenerating them replaces only the Beat-kind ones and never disturbs a cue
//! the editor placed by hand (docs/09-AUDIO.md).
//!
//! In plain terms: a marker is a labelled flag at a moment on the timeline.
//! Some you drop yourself; the beat ones Kiriko works out from the music. When
//! you edit, times can *snap* to the nearest marker so cuts land on the beat.

use crate::time::{CompTime, Rational};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where a marker came from. `Beat` carries a 0..1 confidence (its onset
/// prominence) so weak beats can be filtered or drawn faintly.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum MarkerKind {
    /// Placed by the editor.
    #[default]
    User,
    /// Detected from audio; regeneration replaces only these.
    Beat { confidence: f32 },
    /// A chapter division.
    Chapter,
}

/// A flag at a moment on its owner's timeline (a composition, for now).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: Uuid,
    /// Time on the owner's timebase.
    pub time: CompTime,
    /// A spanning marker's length, if any.
    #[serde(default)]
    pub duration: Option<Rational>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: MarkerKind,
    /// Unknown fields from newer Kiriko versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Marker {
    /// A plain user marker at `time`.
    pub fn user(id: Uuid, time: Rational) -> Self {
        Self {
            id,
            time: CompTime(time),
            duration: None,
            label: String::new(),
            kind: MarkerKind::User,
            extra: serde_json::Map::new(),
        }
    }

    /// A detected beat marker at `time` with `confidence` in 0..1.
    pub fn beat(id: Uuid, time: Rational, confidence: f32) -> Self {
        Self {
            id,
            time: CompTime(time),
            duration: None,
            label: String::new(),
            kind: MarkerKind::Beat { confidence },
            extra: serde_json::Map::new(),
        }
    }

    pub fn is_beat(&self) -> bool {
        matches!(self.kind, MarkerKind::Beat { .. })
    }
}

/// The nearest marker time to `time` within `threshold`, or `time` unchanged if
/// none is close enough — the snap used when editing near markers. Distances
/// are compared in f64; the returned time is the marker's exact rational.
pub fn snap_time(time: Rational, markers: &[Marker], threshold: Rational) -> Rational {
    let t = time.to_f64();
    let th = threshold.to_f64().abs();
    markers
        .iter()
        .map(|m| m.time.0)
        .filter(|mt| (mt.to_f64() - t).abs() <= th)
        .min_by(|a, b| {
            (a.to_f64() - t)
                .abs()
                .partial_cmp(&(b.to_f64() - t).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(time)
}

/// Merge freshly-detected `beats` into `existing`, replacing only the previous
/// Beat-kind markers (user and chapter markers are untouched), sorted by time
/// (docs/impl/beat-detection.md §3, docs/03-DATA-MODEL.md §11).
pub fn with_regenerated_beats(existing: &[Marker], beats: Vec<Marker>) -> Vec<Marker> {
    let mut out: Vec<Marker> = existing.iter().filter(|m| !m.is_beat()).cloned().collect();
    out.extend(beats);
    out.sort_by_key(|m| m.time.0);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    fn at(secs_n: i64, secs_d: i64, kind: MarkerKind) -> Marker {
        Marker {
            id: Uuid::now_v7(),
            time: CompTime(rat(secs_n, secs_d)),
            duration: None,
            label: String::new(),
            kind,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn snap_picks_the_nearest_within_threshold() {
        let markers = [
            at(1, 1, MarkerKind::User),
            at(2, 1, MarkerKind::Beat { confidence: 1.0 }),
        ];
        // 1.02s snaps to the 1s marker (within 50ms).
        assert_eq!(snap_time(rat(51, 50), &markers, rat(1, 20)), rat(1, 1));
        // 1.5s has nothing within 50ms → unchanged.
        assert_eq!(snap_time(rat(3, 2), &markers, rat(1, 20)), rat(3, 2));
        // Exactly between two markers with a wide threshold → the nearer wins
        // deterministically (1.4 is closer to 1 than to 2).
        assert_eq!(snap_time(rat(7, 5), &markers, rat(1, 1)), rat(1, 1));
    }

    #[test]
    fn regenerating_beats_keeps_user_markers() {
        let existing = [
            at(0, 1, MarkerKind::User),
            at(1, 1, MarkerKind::Beat { confidence: 0.5 }),
            at(3, 1, MarkerKind::Chapter),
        ];
        let fresh = vec![
            Marker::beat(Uuid::now_v7(), rat(2, 1), 0.9),
            Marker::beat(Uuid::now_v7(), rat(4, 1), 0.8),
        ];
        let merged = with_regenerated_beats(&existing, fresh);
        // Old beat (1s) gone; user (0s) and chapter (3s) kept; sorted by time.
        let times: Vec<Rational> = merged.iter().map(|m| m.time.0).collect();
        assert_eq!(times, vec![rat(0, 1), rat(2, 1), rat(3, 1), rat(4, 1)]);
        assert_eq!(merged.iter().filter(|m| m.is_beat()).count(), 2);
    }
}
