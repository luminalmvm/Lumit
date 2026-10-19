//! Timeline markers (docs/03-DATA-MODEL.md §11): user cue points, chapters, and
//! automatically-detected beat markers. Beat markers carry provenance so that
//! regenerating them replaces only the Beat-kind ones and never disturbs a cue
//! the editor placed by hand (docs/09-AUDIO.md).
//!
//! In plain terms: a marker is a labelled flag at a moment on the timeline.
//! Some you drop yourself; the beat ones Lumit works out from the music. When
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
    /// Which of the label colours the marker wears, as an index into the
    /// interface's label palette, the same one a layer's label picks from.
    /// `None` is the plain marker grey, which is what every marker made before
    /// markers had a colour opens as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<u8>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
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
            colour: None,
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
            colour: None,
            extra: serde_json::Map::new(),
        }
    }

    pub fn is_beat(&self) -> bool {
        matches!(self.kind, MarkerKind::Beat { .. })
    }
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

/// A marker as a panel hands it back: what it can edit, with the span in
/// whole frames of the owner's rate.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkerEdit {
    pub id: Uuid,
    pub time: CompTime,
    pub label: String,
    pub colour: Option<u8>,
    /// How many frames the marker spans, or `None` for a moment.
    pub duration_frames: Option<i64>,
}

/// A whole marker list coming back from an editor, merged onto the list the
/// document holds.
///
/// **Why a merge and not a conversion.** An edited marker carries what a panel
/// can edit — id, time, label, colour and the span — and the engine's marker
/// carries two more it cannot: the *kind* (a detected beat's provenance, plus
/// its confidence) and the `extra` map that keeps fields a newer Lumit wrote
/// (docs/10 §1.1's forward-compatibility promise). Rebuilding each marker from
/// them alone flattened those to their defaults, so **dragging
/// or renaming a beat marker on the ruler turned it into an ordinary cue** —
/// and then *Clear beat markers* could no longer find it, because there was
/// nothing left to say it had ever been one. The ruler markers made that a
/// click away.
///
/// So each incoming marker is matched by id against what the document already
/// holds: found, and it keeps that marker's kind and extra; new, and it is a
/// plain user marker, which is exactly what a marker the panel just created is.
///
/// **The duration crosses now, and still rides the merge.** It crosses as
/// *frames* — what the ruler draws with — so a straight conversion back would
/// round a marker whose span is finer than a frame down to the frame boundary
/// every time an unrelated drag wrote the list. So a duration that still reads
/// as the same number of frames keeps the document's exact rational, and only a
/// span the panel actually changed is rebuilt from frames.
pub fn merge_edits(
    incoming: Vec<MarkerEdit>,
    existing: &[Marker],
    rate: crate::time::FrameRate,
) -> Vec<Marker> {
    incoming
        .into_iter()
        .map(|m| {
            let was = existing.iter().find(|e| e.id == m.id);
            merge_edit(m, was, rate)
        })
        .collect()
}

fn merge_edit(m: MarkerEdit, existing: Option<&Marker>, rate: crate::time::FrameRate) -> Marker {
    let was = existing.and_then(|e| e.duration);
    let duration = match m.duration_frames {
        // Unchanged in frames: keep the exact span the document holds, so a
        // rename or a drag cannot quantise a marker nobody resized.
        Some(frames) if was.map(|d| rate.frame_at(CompTime(d))) == Some(frames) => was,
        // A span the panel set. Nought frames or fewer is a moment, which is
        // what "no span" means everywhere else.
        Some(frames) if frames > 0 => rate.time_of_frame(frames).ok().map(|t| t.0),
        Some(_) => None,
        None => None,
    };
    Marker {
        id: m.id,
        time: m.time,
        duration,
        label: m.label,
        colour: m.colour,
        // Everything the panel cannot edit is carried from the marker that was
        // already there; a marker it has just made has nothing to carry, and
        // takes the plain-user defaults.
        kind: existing.map(|e| e.kind).unwrap_or_default(),
        extra: existing.map(|e| e.extra.clone()).unwrap_or_default(),
    }
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
            colour: None,
            extra: serde_json::Map::new(),
        }
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

    #[test]
    fn merged_edits_keep_what_the_panel_cannot_see() {
        let rate = crate::time::FrameRate::new(25, 1).unwrap();
        let mut beat = Marker::beat(Uuid::now_v7(), rat(1, 1), 0.7);
        // A span finer than a frame, and a field from a newer Lumit.
        beat.duration = Some(rat(1, 10));
        beat.extra.insert("newer".into(), true.into());
        let same = rate.frame_at(CompTime(rat(1, 10)));
        let edit = |id, duration_frames| MarkerEdit {
            id,
            time: CompTime(rat(2, 1)),
            label: "Drop".into(),
            colour: Some(3),
            duration_frames,
        };
        let new = Uuid::now_v7();
        let merged = merge_edits(
            vec![
                edit(beat.id, Some(same)),
                edit(beat.id, Some(5)),
                edit(beat.id, Some(0)),
                edit(beat.id, None),
                edit(new, Some(5)),
            ],
            std::slice::from_ref(&beat),
            rate,
        );

        let durations: Vec<_> = merged.iter().map(|m| m.duration).collect();
        assert_eq!(
            durations,
            [
                Some(rat(1, 10)),
                Some(rat(1, 5)),
                None,
                None,
                Some(rat(1, 5))
            ]
        );
        for m in &merged[..4] {
            assert_eq!(m.kind, beat.kind);
            assert_eq!(m.extra, beat.extra);
            assert_eq!(
                (m.time, m.label.as_str(), m.colour),
                (CompTime(rat(2, 1)), "Drop", Some(3))
            );
        }
        assert_eq!(merged[4].kind, MarkerKind::User);
        assert!(merged[4].extra.is_empty());
    }
}
