//! Planar track (docs/08 §3.87, K-579): the handle for a flat surface being
//! followed through a shot.
//!
//! **In plain terms.** Something flat in the shot — a phone screen, a poster, a
//! sign, a laptop lid — is to have your own picture put on it. Drop this on the
//! footage, put the four points round the flat thing, press Analyse. The
//! tracker follows the specks *inside* the quad and works out, frame by frame,
//! how that surface is being stretched by the camera. Then **Create corner
//! pin** puts a Corner pin on whichever layer you name, with its four corners
//! keyframed to sit exactly where the surface is on every frame.
//!
//! **Why it is not a mode on Camera track.** The two effects share their first
//! step and nothing after it. A Camera track answers *where the camera was* —
//! one answer for a whole clip, keyed to the media file, read by a Camera
//! layer through a link, with a point cloud and a focal length. A Planar track
//! answers *where this surface is*, which is a property of the quad the user
//! drew and not of the file: two of them on one shot are two different
//! answers, and the second would overwrite the first if they shared a store
//! entry. Folding the two together would make every row of both conditional on
//! a mode, and every reading downstream a union that has to be unwrapped
//! before it can be drawn. docs/08 §4's Tracker row already frames planar
//! tracking as its own thing — "producing keyframed transforms and corner-pin
//! data" — beside, not inside, the camera solve. Two effects; one substrate.
//!
//! **What is not here.** The status readout is not a parameter, for the reason
//! the Camera track's is not: "tracking, frame 214 of 900" is live job state,
//! and a parameter is something the document saves and the timeline animates.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// Feature density's option labels, in index order — the Camera track's own
/// three, meaning the same three things to the same tracker.
pub use super::camera_track::{density, DENSITY, DENSITY_DEFAULT, DENSITY_OPTIONS};

/// The Planar track effect's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "planar_track",
    label = "Planar track",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    // No picture, so no matte — the Camera track's reasoning exactly: this is a
    // handle holding a job, not an image operation.
    matte = false,
)]
pub struct PlanarTrack {
    /// Start the analysis.
    #[action(label = "Analyse")]
    pub analyse: (),
    /// Stop a running analysis.
    #[action(label = "Cancel")]
    pub cancel: (),
    /// Put a Corner pin on [`pin_layer`](Self::pin_layer), keyframed to the
    /// tracked surface. Refused until there is a track to read.
    #[action(label = "Create corner pin")]
    pub pin: (),

    /// px@comp (K-260): the tracked quad's upper-left corner on the reference
    /// frame. The four corners are the *reference* shape — where the surface is
    /// at the start of the shot — and everything the analysis finds is measured
    /// against them.
    ///
    /// The schema defaults are a nominal 1080p rectangle; a fresh instance is
    /// put on the actual comp by
    /// [`instantiate_for_raster`](crate::fx::instantiate_for_raster), because a
    /// schema constant cannot know the raster (§1.2).
    #[slider(label = "Upper left x", min = -1920.0, max = 3840.0, default = 660.0, unit = Px)]
    pub upper_left_x: f32,
    /// px@comp; see [`upper_left_x`](Self::upper_left_x).
    #[slider(label = "Upper left y", min = -1080.0, max = 2160.0, default = 370.0, unit = Px)]
    pub upper_left_y: f32,
    /// px@comp: the quad's upper-right corner on the reference frame.
    #[slider(label = "Upper right x", min = -1920.0, max = 3840.0, default = 1260.0, unit = Px)]
    pub upper_right_x: f32,
    /// px@comp; see [`upper_right_x`](Self::upper_right_x).
    #[slider(label = "Upper right y", min = -1080.0, max = 2160.0, default = 370.0, unit = Px)]
    pub upper_right_y: f32,
    /// px@comp: the quad's lower-left corner on the reference frame.
    #[slider(label = "Lower left x", min = -1920.0, max = 3840.0, default = 660.0, unit = Px)]
    pub lower_left_x: f32,
    /// px@comp; see [`lower_left_x`](Self::lower_left_x).
    #[slider(label = "Lower left y", min = -1080.0, max = 2160.0, default = 710.0, unit = Px)]
    pub lower_left_y: f32,
    /// px@comp: the quad's lower-right corner on the reference frame.
    #[slider(label = "Lower right x", min = -1920.0, max = 3840.0, default = 1260.0, unit = Px)]
    pub lower_right_x: f32,
    /// px@comp; see [`lower_right_x`](Self::lower_right_x).
    #[slider(label = "Lower right y", min = -1080.0, max = 2160.0, default = 710.0, unit = Px)]
    pub lower_right_y: f32,

    /// Which layer **Create corner pin** puts its Corner pin on. Unset offers
    /// nothing to press: a pin with no layer to sit on would be a button that
    /// silently did nothing.
    ///
    /// `self_default` is off, deliberately. Pinning the tracked layer to its own
    /// surface is a corner pin that does very nearly nothing, and it is never
    /// what the gesture is for — the picture that goes on the phone is a
    /// different layer.
    ///
    /// The resolved field is a `bool` — whether the row names a layer at all —
    /// exactly as every other `#[layer]` row's is. The *identity* is read off
    /// the stored [`EffectValue::Layer`](crate::model::EffectValue::Layer),
    /// which is what the corner-pin gesture asks for.
    #[layer(label = "Pin layer", self_default = false)]
    pub pin_layer: bool,

    /// How many features the tracker chases inside the quad: [`DENSITY`]'s grid
    /// and per-bucket counts, shared with the Camera track because it is the
    /// same detector being asked the same question.
    #[choice(options = DENSITY_OPTIONS, default = DENSITY_DEFAULT, label = "Feature density")]
    pub density: u32,
    /// Whether the layer's masks exclude regions from tracking *as well as* the
    /// quad. The quad already says where to look; a mask says what to ignore
    /// inside it — the hand over the phone, the reflection crossing the sign.
    #[toggle(default = true, label = "Use masks")]
    pub use_masks: bool,
}

/// The Planar track's behaviour: none, by design.
pub struct PlanarTrackDef;

impl EffectDef for PlanarTrackDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PlanarTrack as EffectMetadata>::SCHEMA
    }

    /// Identity, exactly as the Camera track is: it holds a job, not a value.
    fn is_image_op(&self) -> bool {
        false
    }
}
