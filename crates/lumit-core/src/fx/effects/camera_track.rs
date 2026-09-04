//! Camera track (docs/08 §3.85): the handle for a camera solve.
//!
//! **In plain terms.** Drop this on the footage you want tracked and press
//! Analyse. The effect itself does nothing to the picture — it is not a look,
//! it is a *button and a readout*. The work happens on its own thread, on the
//! whole unaltered source clip, and what comes back is a solved camera
//! path a Camera layer can be linked to. You keep editing while it runs, which
//! is the working shape After Effects has and the owner asked for.
//!
//! **Why an effect at all.** Because that is where the controls belong: on the
//! layer being tracked, in the stack, with the rest of that layer's settings,
//! rather than in a modal window that owns the application until it finishes.
//! The effect is the handle; the analysis is elsewhere.
//!
//! **What is not here.** The status readout is *not* a parameter. A parameter
//! is something the document stores and the timeline animates, and "solving,
//! frame 214 of 900" is none of those — it is live job state, and it crosses
//! as job state in stage 2. Faking it as a string row would put a progress bar
//! in the save file.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// Feature density's option labels, in index order (Low / Normal / High).
pub const DENSITY_OPTIONS: &[&str] = &["Low", "Normal", "High"];

/// The default Feature density index — Normal, which is
/// `TrackSettings::default()` exactly.
pub const DENSITY_DEFAULT: u32 = 1;

/// What one Feature density option means to the tracker: `(buckets across,
/// buckets down, best-N per bucket)` — the `grid` and `per_bucket` fields of
/// `lumit_track::TrackSettings` (docs/impl/tracking.md §2).
///
/// The table lives here rather than in `lumit-track` because the *choice* is a
/// control on this effect and the crate that owns the control cannot depend on
/// the crate that owns the tracker (docs/05: engine crates, one direction).
/// The analysis job reads it the other way round, which is the only direction
/// there is.
///
/// Normal is the tracker's own default, so the middle option changes nothing —
/// which is what makes the other two honest about being a deliberate move.
pub const DENSITY: [(usize, usize, usize); 3] = [(12, 12, 1), (16, 16, 2), (20, 20, 3)];

/// The detection grid and per-bucket count for a stored Feature density index.
/// An index this build does not know reads as Normal — the tasteful default,
/// never a fault (14-ENGINEERING-RULES §4).
#[must_use]
pub fn density(index: u32) -> (usize, usize, usize) {
    *DENSITY
        .get(index as usize)
        .unwrap_or(&DENSITY[DENSITY_DEFAULT as usize])
}

/// The Camera track effect's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "camera_track",
    label = "Camera track",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    // No picture, so no matte — the Controls family's `None`, for an effect
    // that is a handle rather than an image operation.
    matte = false,
)]
pub struct CameraTrack {
    /// Start the analysis. A button, not a value.
    #[action(label = "Analyse")]
    pub analyse: (),
    /// Stop a running analysis. Live only while one is running, which is job
    /// state and so is the panel's business in stage 3, not the schema's.
    #[action(label = "Cancel")]
    pub cancel: (),
    /// How many features the tracker chases: [`DENSITY`]'s grid and per-bucket
    /// counts. More is slower and more robust; the middle option is the
    /// tracker's own default.
    #[choice(options = DENSITY_OPTIONS, default = DENSITY_DEFAULT, label = "Feature density")]
    pub density: u32,
    /// Whether the layer's masks exclude regions from tracking (the mask
    /// carriage, which the tracker reads). On by default: a mask drawn on a
    /// tracked layer is almost always drawn round the thing that moves.
    #[toggle(default = true, label = "Use masks")]
    pub use_masks: bool,
    /// Whether the solved point cloud draws over the picture on this layer.
    /// On after a solve.
    #[toggle(default = true, label = "Show points")]
    pub show_points: bool,
}

/// The Camera track's behaviour: none, by design.
pub struct CameraTrackDef;

impl EffectDef for CameraTrackDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CameraTrack as EffectMetadata>::SCHEMA
    }

    /// It renders identity. The resolve step pushes no op for it, exactly as it
    /// pushes none for the Controls family — a different reason for the same
    /// honest answer: this one holds a *job*, not a value.
    fn is_image_op(&self) -> bool {
        false
    }
}
