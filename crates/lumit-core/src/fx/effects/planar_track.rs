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
//! **The same four corners are also a transform.** Where their centre went is a
//! position, how far they turned is a rotation, how much they grew is a scale —
//! so **Create transform keys** writes that movement onto the named layer's own
//! Position, Rotation and Scale instead, added to whatever it already had.
//!
//! **And not everything worth following is flat.** A light on a car, a badge on
//! a shoulder, two marks on opposite walls of a room: [`Follow`](PlanarTrack::follow)
//! turns the effect to **one point** or **two points**, each a small box
//! followed on its own. One box gives a position; two give a position, a turn
//! and a growth from the line between them — and being separate boxes, they need
//! no relation to each other at all (K-735).
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

/// The **Follow** row's option labels, in index order: what the analysis is
/// asked to follow, and therefore what shape of answer it can give.
///
/// *Surface* is a homography over the quad — eight numbers, and the only one of
/// the three that can produce a real Corner pin. *One point* is one small box,
/// followed as a slide. *Two points* is two boxes, each followed on its own,
/// read together as a slide, a turn and a growth. Neither point option assumes
/// the boxes are on one plane, or on one object.
pub const FOLLOW_OPTIONS: &[&str] = &["Surface", "One point", "Two points"];

/// The default **Follow** index — the surface, which is what the effect was.
pub const FOLLOW_DEFAULT: u32 = 0;

/// The **Follow** index meaning one point: the only one whose answer carries no
/// turn and no growth, and so the only one that writes Position alone.
pub const FOLLOW_ONE_POINT: u32 = 1;

/// The **Follow** index meaning two points.
pub const FOLLOW_TWO_POINTS: u32 = 2;

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
    /// Keyframe [`pin_layer`](Self::pin_layer)'s own Position — and, unless
    /// [`follow`](Self::follow) is one point, its Rotation and Scale — to the
    /// movement the track measured. Refused until there is a track to read.
    #[action(label = "Create transform keys")]
    pub transform_keys: (),

    /// What the analysis follows: [`FOLLOW_OPTIONS`]. *Surface* reads the four
    /// corner rows below; the two point options read
    /// [`point1_x`](Self::point1_x), [`point2_x`](Self::point2_x) and
    /// [`region`](Self::region) instead, and the corner rows are left alone
    /// rather than repurposed — the two geometries mean different things and a
    /// row that changed its meaning under a dropdown is the kind of control
    /// nobody trusts twice.
    #[choice(options = FOLLOW_OPTIONS, default = FOLLOW_DEFAULT, label = "Follow")]
    pub follow: u32,

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

    /// px@comp (K-260): the first tracked point on the reference frame, read
    /// when [`follow`](Self::follow) is one of the point options.
    #[slider(label = "Point 1 x", min = -1920.0, max = 3840.0, default = 860.0, unit = Px)]
    pub point1_x: f32,
    /// px@comp; see [`point1_x`](Self::point1_x).
    #[slider(label = "Point 1 y", min = -1080.0, max = 2160.0, default = 440.0, unit = Px)]
    pub point1_y: f32,
    /// px@comp: the second tracked point, read only when
    /// [`follow`](Self::follow) is *Two points*. It is followed entirely on its
    /// own, so it need not be on the same surface — or the same object — as the
    /// first.
    #[slider(label = "Point 2 x", min = -1920.0, max = 3840.0, default = 1060.0, unit = Px)]
    pub point2_x: f32,
    /// px@comp; see [`point2_x`](Self::point2_x).
    #[slider(label = "Point 2 y", min = -1080.0, max = 2160.0, default = 640.0, unit = Px)]
    pub point2_y: f32,

    /// How wide each point's search box is, in px@comp: the box is this across,
    /// centred on the point.
    ///
    /// Wide enough to hold some texture, narrow enough to hold only the thing
    /// being followed — a box that reaches past the badge onto the moving
    /// shoulder behind it is asking two objects the same question.
    #[slider(label = "Region size", min = 16.0, max = 400.0, default = 80.0, unit = Px)]
    pub region: f32,

    /// Which layer **Create corner pin** puts its Corner pin on, and which
    /// layer **Create transform keys** keyframes. Unset offers nothing to
    /// press: a pin with no layer to sit on would be a button that silently did
    /// nothing.
    ///
    /// `self_default` is off, deliberately. Pinning the tracked layer to its own
    /// surface is a corner pin that does very nearly nothing, and it is never
    /// what the gesture is for — the picture that goes on the phone is a
    /// different layer.
    ///
    /// One row for both gestures rather than two: it names the layer the track
    /// is being spent on, and a second row would be a second place for the same
    /// answer to be wrong.
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
