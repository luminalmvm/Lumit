//! Finding the beat in a composition's audio, and marking it.
//!
//! # In plain terms
//!
//! Cutting to music means knowing where the beats are. This listens to
//! everything audible in a composition, finds the moments that sound like hits,
//! works out the tempo, nudges the near-misses onto that grid, and drops a
//! marker on each — so the Timeline's snapping has something true to snap to
//! (docs/09 §5).
//!
//! **Detection replaces only the beat markers.** Chapter marks and anything
//! typed by hand survive, because re-running detection at a different
//! sensitivity is an ordinary thing to do and losing your own notes to it would
//! not be.
//!
//! It can take a few seconds on a long comp — it mixes the audio and analyses
//! the lot — so detection is deliberately NOT `#[frb(sync)]`: it runs off
//! Dart's own thread and the interface never waits on it. The markers land as
//! one committed op when it finishes, and the change stream repaints the panels
//! exactly as any other edit does.
//!
//! The seconds themselves are spent on the **beat worker** ([`crate::beats`]),
//! not here. flutter_rust_bridge would otherwise run the analysis on the pool
//! it keeps for asynchronous calls, which every panel's reads share: a couple
//! of detections at once sat on the whole pool and stopped them. This call now
//! hands the job over and waits for its own answer, so it still returns the
//! count it always did.

use flutter_rust_bridge::frb;

use crate::api::{composition::CompositionReference, BridgeError};

/// The Audio panel's Beats section, as one crossing (docs/09 §5, the approved
/// AudioWorkspace board): what to listen to, how keenly, where, and the grid.
///
/// Every field has a "just detect" default — [`BridgeBeatOptions::standard`] —
/// so the Timeline toolbar's one-click detection and the panel's tuned run are
/// the same call.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeBeatOptions {
    /// The layer to listen to, by id, or an empty string for the comp's own
    /// mix — the same two shapes the Audio level driver reads. A named layer
    /// that is gone or silent finds nothing, which is an answer of zero.
    pub source_layer: String,
    /// 0..100, 50 the standard setting, higher finds more.
    pub sensitivity_percent: u32,
    /// Keep only the beats inside the comp's work area. A comp with no work
    /// area analyses the lot — the range control's honest degrade.
    pub work_area_only: bool,
    /// Drop a beat closer than this to a louder neighbour, in milliseconds.
    /// Zero keeps everything the detector found.
    pub min_spacing_ms: u32,
    /// Snap to this tempo instead of the estimate — the BPM well and Tap.
    /// Zero (or negative) means "use the estimate", never "no grid".
    pub bpm_override: f64,
    /// Nudge every generated marker by this many milliseconds — the panel's
    /// phase chips, for a grid that is right but early or late.
    pub phase_ms: f64,
}

impl BridgeBeatOptions {
    /// One-click detection: the comp's mix, standard sensitivity, the whole
    /// comp, no spacing floor, the estimated tempo, no nudge.
    #[frb(sync)]
    pub fn standard() -> BridgeBeatOptions {
        BridgeBeatOptions {
            source_layer: String::new(),
            sensitivity_percent: 50,
            work_area_only: false,
            min_spacing_ms: 0,
            bpm_override: 0.0,
            phase_ms: 0.0,
        }
    }
}

/// What a detection run found: how many markers landed, and the tempo the
/// grid used — the estimate, or the override handed in — for the BPM well.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeBeatsResult {
    pub placed: u32,
    pub bpm: f64,
}

/// The comp's confirmed beat grid (docs/09 §5, K-698): what the last
/// detection ran its grid at, for the Timeline's beat band to number bars
/// from. Bars are the grid read four beats at a time.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeBeatGrid {
    /// Beats per minute; always positive on a grid that exists.
    pub bpm: f64,
    /// Where beat zero falls, in comp seconds.
    pub phase_seconds: f64,
}

impl CompositionReference {
    /// Detect beats and replace this comp's beat markers.
    ///
    /// Returns how many markers were placed and the tempo used — zero placed
    /// is a legitimate answer for quiet or arrhythmic audio, and worth showing
    /// as such rather than as a failure. Seconds-long on a long comp, which is
    /// why the analysis itself happens on the beat worker ([`crate::beats`])
    /// and this call waits for it.
    pub fn detect_beats(
        &self,
        options: BridgeBeatOptions,
    ) -> Result<BridgeBeatsResult, BridgeError> {
        let composition = self.composition()?;
        let document = {
            let state = self.project()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };

        let phase_ms = options.phase_ms;

        // The mixdown and the onset analysis, off this thread — and never with
        // the project lock held, which is why the snapshot above is taken and
        // let go before anything heavy starts (docs/14 §3).
        let found =
            crate::beats::detect(document, self.id, composition.duration.0.to_f64(), options)?;

        // The markers are minted here, not by the worker: an id is not part of
        // what the analysis found, and keeping it out of the answer is what
        // lets "the same audio finds the same beats" be a checkable claim.
        let beats: Vec<lumit_core::markers::Marker> = found
            .beats
            .iter()
            .filter_map(|beat| {
                let time = lumit_core::Rational::from_f64_on_grid(beat.time_seconds.max(0.0), 1000)
                    .ok()?;
                Some(lumit_core::markers::Marker::beat(
                    uuid::Uuid::now_v7(),
                    time,
                    beat.confidence,
                ))
            })
            .collect();
        let placed = beats.len() as u32;

        let markers = lumit_core::markers::with_regenerated_beats(&composition.markers, beats);

        // The grid the run confirmed (K-698): the tempo used — estimate or
        // override — and the phase nudge, kept on the comp so the Timeline's
        // beat band can number bars. A run that found no tempo clears it: a
        // band numbering bars off a grid the audio no longer answers to would
        // be the panel making the tempo up.
        let grid = (found.bpm > 0.0 && placed > 0)
            .then(|| {
                lumit_core::Rational::from_f64_on_grid(phase_ms / 1000.0, 1000)
                    .ok()
                    .map(|phase| lumit_core::model::BeatGrid {
                        bpm: found.bpm,
                        phase,
                    })
            })
            .flatten();
        self.commit_markers_and_grid(markers, grid)?;
        Ok(BridgeBeatsResult {
            placed,
            bpm: found.bpm,
        })
    }

    /// The comp's confirmed beat grid (K-698), or `None` while no detection
    /// with a tempo has run — what the beat band numbers bars from.
    #[frb(sync)]
    pub fn get_beat_grid(&self) -> Result<Option<BridgeBeatGrid>, BridgeError> {
        Ok(self.composition()?.beat_grid.map(|g| BridgeBeatGrid {
            bpm: g.bpm,
            phase_seconds: g.phase.to_f64(),
        }))
    }

    /// Remove every detected beat marker, keeping the ones a person made —
    /// and the confirmed grid with them, since it described the set that went.
    ///
    /// A comp with none is a calm no-op rather than an error — clearing twice is
    /// something a user does without thinking about it.
    #[frb(sync)]
    pub fn clear_beat_markers(&self) -> Result<(), BridgeError> {
        let composition = self.composition()?;
        let kept: Vec<_> = composition
            .markers
            .iter()
            .filter(|m| !matches!(m.kind, lumit_core::markers::MarkerKind::Beat { .. }))
            .cloned()
            .collect();
        if kept.len() == composition.markers.len() && composition.beat_grid.is_none() {
            return Ok(());
        }
        self.commit_markers_and_grid(kept, None)
    }

    /// One undo step for the pair (K-698): the markers and the grid change
    /// together — detection writes both, clearing takes both away — and two
    /// steps would leave `Ctrl+Z` a state nobody was ever shown.
    #[frb(ignore)]
    fn commit_markers_and_grid(
        &self,
        markers: Vec<lumit_core::markers::Marker>,
        grid: Option<lumit_core::model::BeatGrid>,
    ) -> Result<(), BridgeError> {
        let state = self.project()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state.store.begin_undo_group();
        let outcome = (|| {
            state
                .store
                .commit(lumit_core::Op::SetCompMarkers {
                    comp: self.id,
                    markers,
                })
                .map_err(BridgeError::OpError)?;
            state
                .store
                .commit(lumit_core::Op::SetBeatGrid {
                    comp: self.id,
                    grid,
                })
                .map_err(BridgeError::OpError)?;
            Ok(())
        })();
        state.store.end_undo_group();
        outcome
    }
}
