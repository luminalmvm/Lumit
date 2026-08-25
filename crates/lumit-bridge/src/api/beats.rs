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

impl CompositionReference {
    /// Detect beats and replace this comp's beat markers.
    ///
    /// `sensitivity_percent` runs 0..100, where 50 is the standard setting and
    /// higher finds more. Returns how many markers were placed — zero is a
    /// legitimate answer for quiet or arrhythmic audio, and worth showing as
    /// such rather than as a failure. Seconds-long on a long comp, which is why
    /// the analysis itself happens on the beat worker ([`crate::beats`]) and
    /// this call waits for it.
    pub fn detect_beats(&self, sensitivity_percent: u32) -> Result<u32, BridgeError> {
        let composition = self.composition()?;
        let document = {
            let state = self.project()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };

        // The mixdown and the onset analysis, off this thread — and never with
        // the project lock held, which is why the snapshot above is taken and
        // let go before anything heavy starts (docs/14 §3).
        let found = crate::beats::detect(
            document,
            self.id,
            composition.duration.0.to_f64(),
            sensitivity_percent,
        )?;

        // The markers are minted here, not by the worker: an id is not part of
        // what the analysis found, and keeping it out of the answer is what
        // lets "the same audio finds the same beats" be a checkable claim.
        let beats: Vec<lumit_core::markers::Marker> = found
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
        self.commit_markers(markers)?;
        Ok(placed)
    }

    /// Remove every detected beat marker, keeping the ones a person made.
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
        if kept.len() == composition.markers.len() {
            return Ok(());
        }
        self.commit_markers(kept)
    }

    #[frb(ignore)]
    fn commit_markers(&self, markers: Vec<lumit_core::markers::Marker>) -> Result<(), BridgeError> {
        let state = self.project()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(lumit_core::Op::SetCompMarkers {
                comp: self.id,
                markers,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }
}
