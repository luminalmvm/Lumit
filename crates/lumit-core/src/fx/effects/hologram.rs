//! Hologram: the layer becomes a volume hologram on a flat film. Tip the film
//! and the picture inside it moves with real depth, in one replay colour.
//!
//! The layer is the recorded subject: its colour is what was recorded, and its
//! alpha is the film's outline, so a round sticker is a round layer. A Depth
//! map layer pushes each part of the subject behind the film (white is near);
//! without one the subject is a flat picture at Subject depth. Behind it sits
//! a recorded grid or star field, or any layer, and the film can carry fine
//! line patterns of its own that split light into rainbows.
//!
//! Each light replays the picture along the direction the hologram's grating
//! sends it: in one Bragg colour that turns bluer at steep angles for a
//! reflection hologram, or in a colour that follows the eye up and down for a
//! rainbow hologram. The image fades as a light leaves the reference
//! direction, and its deep parts soften under a large light. The film itself
//! shares Holographic foil's sheet, lights, studio, coat and output.
//!
//! GPU only: `apply_cpu` keeps its identity default.
//!
//! The model, its constants and its controls follow holospot by Dmitry Kurash
//! (MIT, Copyright (c) 2026 Dmitry Kurash), reimplemented here, with the
//! recorded volume reduced to a picture and a depth map.

use crate::fx::{
    builtins::MAP_PLACEMENT_OPTIONS, params::ParamId, params::Params, params::Value,
    presets::EffectPreset, registry::EffectDef, registry::EffectMetadata, registry::ResolveCx,
    schema::EffectSchema,
};
use crate::model::EffectInstance;
use lumit_fx_macros::Effect;

use super::holographic_foil::{motion, BLOCKS};

mod groups;
mod pack;
mod presets;

const SPECTRUM_OPTIONS: &[&str] = &["Equal", "Daylight", "Tungsten", "LED", "Fluorescent"];

/// The Type choice for a rainbow hologram.
const RAINBOW: u32 = 1;

/// The Recording choice for a stereogram.
const STEREOGRAM: u32 = 1;

/// Hologram's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "hologram",
    label = "Hologram",
    version = 1,
    category = Stylise,
    // Two traces through the depth map a pixel, and the film's own lights.
    cost = Heavy,
    roi = Exact,
    // Not seeded: layer time joins the frame's key only while the film
    // moves with it (`moves_with_time`).
    premultiplied = true,
    groups = groups::HOLOGRAM_GROUPS,
    enabled_when = groups::HOLOGRAM_ENABLED_WHEN,
    matte = (
        "matte",
        "scales the replay per pixel: white keeps the image, black leaves bare film",
    ),
)]
pub struct Hologram {
    /// A thick reflection hologram in one colour, or an embossed rainbow
    /// hologram on a mirror.
    #[choice(label = "Type", options = ["Reflection", "Rainbow"], default = 0)]
    pub hologram_type: u32,

    /// How much of the light the film sends into the image, per cent.
    #[slider(min = 5.0, max = 100.0, default = 85.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub efficiency: f32,

    /// How strongly the film's mirror shows the lights, per cent. 100 is
    /// physical.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub glare: f32,

    /// How the layer's colour reaches the image: as brightness in the replay
    /// colour, multiplied by the replay colour, or in its own colours.
    #[choice(
        options = ["Replay colour only", "Tinted by the layer", "Full colour"],
        default = 0
    )]
    pub subject_colour: u32,

    /// How far behind the film the subject sits, millimetres. Above 0 it
    /// stands in front.
    #[slider(label = "Subject depth (mm)", min = -40.0, max = 15.0, default = -8.0, hard_min = -500.0, hard_max = 500.0, unit = Raw)]
    pub subject_depth: f32,

    /// A layer whose brightness is the subject's depth, white near. Decided
    /// by the render, so always `false` here.
    #[layer(label = "Depth map", self_default = false)]
    pub depth_map: bool,

    /// Where the Depth map layer's picture sits on this one.
    #[choice(label = "Placement", options = *MAP_PLACEMENT_OPTIONS, default = 1)]
    pub depth_map_placement: u32,

    /// How far the Depth map's black lies behind its white, millimetres.
    #[slider(label = "Depth scale (mm)", min = 0.0, max = 40.0, default = 10.0, hard_min = 0.0, hard_max = 500.0, unit = Raw)]
    pub depth_scale: f32,

    /// Reads the Depth map with black near.
    #[toggle(label = "Invert", default = false)]
    pub depth_invert: bool,

    /// What was recorded behind the subject: a grid or stars, or line
    /// patterns on the film that split light into colour.
    #[choice(
        options = [
            "None", "Grid", "Stars", "Seal", "Rosette", "Spiral", "Lattice", "Sunburst",
            "Sparkle"
        ],
        dividers_after = &[0, 2],
        default = 1
    )]
    pub backdrop: u32,

    /// How far behind the film the grid, the stars and the Backdrop layer
    /// sit, millimetres.
    #[slider(label = "Backdrop depth (mm)", min = -80.0, max = -5.0, default = -30.0, hard_min = -1000.0, hard_max = 0.0, unit = Raw)]
    pub backdrop_depth: f32,

    /// A layer recorded behind the subject. Decided by the render, so always
    /// `false` here.
    #[layer(label = "Backdrop layer", self_default = false)]
    pub backdrop_layer: bool,

    /// Where the Backdrop layer's picture sits on this one.
    #[choice(label = "Placement", options = *MAP_PLACEMENT_OPTIONS, default = 1)]
    pub backdrop_layer_placement: u32,

    /// How far behind the surface the film's line patterns lie, millimetres.
    #[slider(label = "Pattern depth (mm)", min = -20.0, max = 0.0, default = -3.0, hard_min = -200.0, hard_max = 0.0, unit = Raw)]
    pub film_depth: f32,

    /// How wide the patterns and the grid are, millimetres.
    #[slider(label = "Pattern size (mm)", min = 5.0, max = 80.0, default = 28.0, hard_min = 1.0, hard_max = 1000.0, unit = Raw)]
    pub pattern_size: f32,

    /// How much of each line spacing a pattern's line fills, per cent.
    #[slider(label = "Line width", min = 5.0, max = 60.0, default = 22.0, hard_min = 1.0, hard_max = 100.0, unit = Percent)]
    pub line_width: f32,

    /// The groove spacing inside the pattern lines, nanometres.
    #[slider(
        label = "Pitch (nm)",
        min = 600.0,
        max = 1600.0,
        default = 1000.0,
        hard_min = 200.0,
        hard_max = 5000.0,
        unit = Raw
    )]
    pub pitch: f32,

    /// How strongly the pattern lines split light into colour.
    #[slider(label = "Strength", min = 0.0, max = 3.0, default = 1.0, hard_min = 0.0, hard_max = 10.0, unit = Raw)]
    pub pattern_strength: f32,

    /// The colour the image replays in when lit from the reference
    /// direction, nanometres.
    #[slider(label = "Colour (nm)", min = 450.0, max = 650.0, default = 545.0, hard_min = 380.0, hard_max = 780.0, unit = Raw)]
    pub colour: f32,

    /// Where the light must be for the clean image, as seen from the film:
    /// -1 to 1 across.
    #[slider(label = "Reference x", min = -1.0, max = 1.0, default = -0.5, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub reference_x: f32,

    /// Where the light must be for the clean image, as seen from the film:
    /// -1 to 1 up.
    #[slider(label = "Reference y", min = -1.0, max = 1.0, default = 0.42, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub reference_y: f32,

    /// How wide the reflection hologram's colour band is, nanometres.
    #[slider(label = "Bandwidth (nm)", min = 4.0, max = 60.0, default = 30.0, hard_min = 1.0, hard_max = 200.0, unit = Raw)]
    pub bandwidth: f32,

    /// How far a light can stray from the reference before the reflection
    /// hologram's image fades, degrees.
    #[slider(min = 2.0, max = 40.0, default = 16.0, hard_min = 0.5, hard_max = 90.0, unit = Degrees)]
    pub selectivity: f32,

    /// How wide the rainbow hologram's slit was, degrees. Narrower gives
    /// purer colour.
    #[slider(min = 1.0, max = 20.0, default = 6.0, hard_min = 0.2, hard_max = 60.0, unit = Degrees)]
    pub slit: f32,

    /// One continuous recording, or a stereogram of separate views.
    #[choice(options = ["Optical", "Stereogram"], default = 0)]
    pub recording: u32,

    /// How grainy the laser light makes the image, per cent.
    #[slider(min = 0.0, max = 100.0, default = 25.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub speckle: f32,

    /// How many views the stereogram was made from.
    #[counter(min = 6, max = 64, default = 20, hard_min = 2, hard_max = 256, unit = Raw)]
    pub views: i32,

    /// How softly one view hands over to the next, per cent.
    #[slider(label = "View blend", min = 0.0, max = 100.0, default = 25.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub view_blend: f32,

    /// How wide a fan of angles the views cover, degrees.
    #[slider(label = "View zone", min = 5.0, max = 60.0, default = 30.0, hard_min = 1.0, hard_max = 170.0, unit = Degrees)]
    pub view_zone: f32,

    /// Tips the top edge toward the viewer, degrees. Turned past edge-on,
    /// the film shows its plain back.
    #[slider(
        min = -60.0,
        max = 60.0,
        default = 0.0,
        hard_min = -89.0,
        hard_max = 89.0,
        unit = Degrees
    )]
    pub tilt_x: f32,

    /// Turns the face toward the right, degrees. Turned past edge-on, the
    /// film shows its plain back.
    #[slider(
        min = -60.0,
        max = 60.0,
        default = 0.0,
        hard_min = -89.0,
        hard_max = 89.0,
        unit = Degrees
    )]
    pub tilt_y: f32,

    /// Turns the backdrop and the patterns within the film, degrees.
    #[slider(min = -180.0, max = 180.0, default = 0.0, unit = Degrees)]
    pub spin: f32,

    /// How far the film wobbles on its own, degrees.
    #[slider(min = 0.0, max = 25.0, default = 0.0, hard_min = 0.0, hard_max = 90.0, unit = Degrees)]
    pub auto_tilt: f32,

    /// How many times a second the wobble swings.
    #[slider(
        label = "Auto tilt speed (Hz)",
        min = 0.0,
        max = 2.0,
        default = 0.25,
        hard_min = 0.0,
        hard_max = 30.0,
        unit = Raw
    )]
    pub auto_tilt_speed: f32,

    /// Adds a 3D layer's own turn, its parents' and the comp camera's view
    /// to the tilt. A 2D layer's rotation leaves the tilt alone.
    #[choice(label = "Follow layer rotation", options = ["Off", "On"], default = 0)]
    pub follow_rotation: u32,

    /// How much of each frame the film's colours blur over while it or the
    /// lights move, per cent of the frame. 0 is off.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub motion_blur: f32,

    /// How wide the film is in the world, millimetres.
    #[slider(
        label = "Sheet width (mm)",
        min = 20.0,
        max = 1000.0,
        default = 90.0,
        hard_min = 1.0,
        unit = Raw
    )]
    pub sheet_width: f32,

    /// How far the eye is from the film, metres.
    #[slider(
        label = "View distance (m)",
        min = 0.1,
        max = 5.0,
        default = 0.35,
        hard_min = 0.05,
        unit = Raw
    )]
    pub view_distance: f32,

    /// A ready rig, or Custom to set each light. Custom's rows start as
    /// Studio.
    #[choice(
        options = [
            "Studio", "Spot", "Softbox", "Strip", "Dark", "Desk", "Office", "Raking", "Custom"
        ],
        dividers_after = &[7],
        default = 1
    )]
    pub lighting: u32,

    /// Turns every light and the studio about the view axis, degrees.
    #[slider(min = -180.0, max = 180.0, default = 0.0, unit = Degrees)]
    pub rig_rotation: f32,

    /// A layer the film mirrors. Decided by the render, so always `false`
    /// here.
    #[layer(label = "Environment", self_default = false)]
    pub environment: bool,

    /// How much of what the film mirrors is the Environment layer. At 100 it
    /// takes the studio's place; above, it is brighter.
    #[slider(label = "Amount", min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, hard_max = 1000.0, unit = Percent)]
    pub environment_amount: f32,

    /// Spot A lights the film.
    #[toggle(label = "On", default = true)]
    pub spot_a_on: bool,

    /// Where spot A is, as seen from the film: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.38, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_a_x: f32,

    /// Where spot A is, as seen from the film: -1 to 1 up.
    #[slider(label = "Direction y", min = -1.0, max = 1.0, default = 0.57, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_a_y: f32,

    /// How wide spot A looks from the film, degrees.
    #[slider(label = "Size", min = 0.2, max = 10.0, default = 1.5, hard_min = 0.05, hard_max = 45.0, unit = Degrees)]
    pub spot_a_size: f32,

    /// How bright spot A is.
    #[slider(label = "Intensity", min = 0.0, max = 5000.0, default = 970.0, hard_min = 0.0, unit = Raw)]
    pub spot_a_intensity: f32,

    /// How far spot A is from the film, metres.
    #[slider(label = "Distance (m)", min = 0.3, max = 3.0, default = 1.0, hard_min = 0.05, unit = Raw)]
    pub spot_a_distance: f32,

    /// The colour of spot A's light.
    #[choice(label = "Spectrum", options = *SPECTRUM_OPTIONS, default = 0)]
    pub spot_a_spectrum: u32,

    /// Spot B lights the film.
    #[toggle(label = "On", default = true)]
    pub spot_b_on: bool,

    /// Where spot B is, as seen from the film: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.12, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_b_x: f32,

    /// Where spot B is, as seen from the film: -1 to 1 up.
    #[slider(label = "Direction y", min = -1.0, max = 1.0, default = -0.17, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_b_y: f32,

    /// How wide spot B looks from the film, degrees.
    #[slider(label = "Size", min = 0.2, max = 10.0, default = 2.5, hard_min = 0.05, hard_max = 45.0, unit = Degrees)]
    pub spot_b_size: f32,

    /// How bright spot B is.
    #[slider(label = "Intensity", min = 0.0, max = 5000.0, default = 485.0, hard_min = 0.0, unit = Raw)]
    pub spot_b_intensity: f32,

    /// How far spot B is from the film, metres.
    #[slider(label = "Distance (m)", min = 0.3, max = 3.0, default = 1.2, hard_min = 0.05, unit = Raw)]
    pub spot_b_distance: f32,

    /// The colour of spot B's light.
    #[choice(label = "Spectrum", options = *SPECTRUM_OPTIONS, default = 0)]
    pub spot_b_spectrum: u32,

    /// Spot C lights the film.
    #[toggle(label = "On", default = true)]
    pub spot_c_on: bool,

    /// Where spot C is, as seen from the film: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.42, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_c_x: f32,

    /// Where spot C is, as seen from the film: -1 to 1 up.
    #[slider(label = "Direction y", min = -1.0, max = 1.0, default = 0.71, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_c_y: f32,

    /// How wide spot C looks from the film, degrees.
    #[slider(label = "Size", min = 0.2, max = 10.0, default = 1.0, hard_min = 0.05, hard_max = 45.0, unit = Degrees)]
    pub spot_c_size: f32,

    /// How bright spot C is.
    #[slider(label = "Intensity", min = 0.0, max = 5000.0, default = 970.0, hard_min = 0.0, unit = Raw)]
    pub spot_c_intensity: f32,

    /// How far spot C is from the film, metres.
    #[slider(label = "Distance (m)", min = 0.3, max = 3.0, default = 1.0, hard_min = 0.05, unit = Raw)]
    pub spot_c_distance: f32,

    /// The colour of spot C's light.
    #[choice(label = "Spectrum", options = *SPECTRUM_OPTIONS, default = 0)]
    pub spot_c_spectrum: u32,

    /// The panel lights the film.
    #[toggle(label = "On", default = true)]
    pub panel_on: bool,

    /// Where the panel is, as seen from the film: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.43, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub panel_x: f32,

    /// Where the panel is, as seen from the film: -1 to 1 up.
    #[slider(label = "Direction y", min = -1.0, max = 1.0, default = 0.48, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub panel_y: f32,

    /// How wide the panel looks along the line out from the view axis,
    /// degrees.
    #[slider(label = "Width", min = 1.0, max = 80.0, default = 14.0, hard_min = 0.5, hard_max = 120.0, unit = Degrees)]
    pub panel_width: f32,

    /// How long the panel looks across that line, degrees.
    #[slider(label = "Length", min = 1.0, max = 90.0, default = 50.0, hard_min = 0.5, hard_max = 120.0, unit = Degrees)]
    pub panel_length: f32,

    /// How bright the panel is.
    #[slider(label = "Intensity", min = 0.0, max = 50.0, default = 1.6, hard_min = 0.0, unit = Raw)]
    pub panel_intensity: f32,

    /// How far the panel is from the film, metres.
    #[slider(label = "Distance (m)", min = 0.3, max = 3.0, default = 0.9, hard_min = 0.05, unit = Raw)]
    pub panel_distance: f32,

    /// The colour of the panel's light.
    #[choice(label = "Spectrum", options = *SPECTRUM_OPTIONS, default = 0)]
    pub panel_spectrum: u32,

    /// Two thin strips low at the sides, for the edges to catch.
    #[slider(min = 0.0, max = 10.0, default = 0.5, hard_min = 0.0, unit = Raw)]
    pub rims: f32,

    /// A broad soft light overhead.
    #[slider(min = 0.0, max = 3.0, default = 0.25, hard_min = 0.0, unit = Raw)]
    pub fill: f32,

    /// A pale sheet just above the viewer, so the bare film reads grey.
    #[slider(min = 0.0, max = 2.0, default = 0.2, hard_min = 0.0, unit = Raw)]
    pub scrim: f32,

    /// The faint light everywhere else in front.
    #[slider(min = 0.0, max = 0.5, default = 0.04, hard_min = 0.0, unit = Raw)]
    pub room: f32,

    /// How rough the clear coat is: low is a sharp gloss.
    #[slider(min = 0.01, max = 0.5, default = 0.01, hard_min = 0.005, hard_max = 1.0, unit = Raw)]
    pub coat_roughness: f32,

    /// The fine relief pressed into the surface. A Height map layer takes its
    /// place.
    #[choice(
        label = "Relief",
        options = [
            "Scratches", "Brushed", "Hammered", "Crinkled", "Linen", "Fingerprint", "Sandblast",
            "None"
        ],
        dividers_after = &[6],
        default = 0
    )]
    pub relief_source: u32,

    /// How strongly the relief bends the surface.
    #[slider(label = "Strength", min = 0.0, max = 3.0, default = 0.78, hard_min = 0.0, hard_max = 10.0, unit = Raw)]
    pub relief: f32,

    /// How many times the relief repeats across the film. Higher is finer.
    #[slider(label = "Scale", min = 1.0, max = 12.0, default = 3.5, hard_min = 0.1, hard_max = 100.0, unit = Raw)]
    pub relief_scale: f32,

    /// Which layer the relief bends: the mirror behind the film, or only the
    /// varnish on top.
    #[choice(label = "Relief on", options = ["Foil", "Varnish"], default = 0)]
    pub relief_on: u32,

    /// Turns grooves into ridges.
    #[toggle(label = "Invert", default = true)]
    pub relief_invert: bool,

    /// How far broad waves in the film tip it, degrees.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 10.0, unit = Degrees)]
    pub waviness: f32,

    /// How rough the varnish's fine ripple is, micrometres.
    #[slider(label = "Orange peel (µm)", min = 0.0, max = 2.0, default = 0.0, hard_min = 0.0, hard_max = 20.0, unit = Raw)]
    pub orange_peel: f32,

    /// Fine lines embossed across the film, on the layer Relief is on.
    #[choice(options = ["None", "Lines", "Crosshatch"], default = 0)]
    pub lines: u32,

    /// The spacing of the embossed lines, millimetres.
    #[slider(label = "Line pitch (mm)", min = 0.15, max = 2.0, default = 1.17, hard_min = 0.01, hard_max = 20.0, unit = Raw)]
    pub line_pitch: f32,

    /// How deep the embossed lines are, micrometres.
    #[slider(label = "Line height (µm)", min = 0.0, max = 15.0, default = 0.5, hard_min = 0.0, hard_max = 100.0, unit = Raw)]
    pub line_height: f32,

    /// How far the film bows in its middle, millimetres.
    #[slider(label = "Bow (mm)", min = 0.0, max = 2.0, default = 0.0, hard_min = -20.0, hard_max = 20.0, unit = Raw)]
    pub bow: f32,

    /// Whether the film bows across its width or along its height.
    #[choice(label = "Bow axis", options = ["Across", "Along"], default = 0)]
    pub bow_axis: u32,

    /// Handling marks: smudges, hairline scratches round the highlights, dust
    /// and patchy replay, per cent.
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub wear: f32,

    /// A layer whose brightness is the surface's height. Decided by the
    /// render, so always `false` here.
    #[layer(label = "Height map", self_default = false)]
    pub height_map: bool,

    /// Where the Height map layer's picture sits on this one.
    #[choice(label = "Placement", options = *MAP_PLACEMENT_OPTIONS, default = 1)]
    pub height_map_placement: u32,

    /// How many times the Height map repeats across the film.
    #[slider(label = "Repeats", min = 1.0, max = 12.0, default = 1.0, hard_min = 1.0, hard_max = 100.0, unit = Raw)]
    pub height_map_scale: f32,

    /// How the Height map meets itself where it repeats: as it is, mirrored
    /// so a picture that does not tile has no seam, or held at its edges.
    #[choice(label = "Wrap", options = ["Repeat", "Mirror", "Clamp"], default = 0)]
    pub height_map_wrap: u32,

    /// A gain on the light, stops.
    #[slider(min = -3.0, max = 3.0, default = 1.0, unit = Raw)]
    pub exposure: f32,

    /// How light above white is rolled off. Off keeps it all, for Glow.
    #[choice(options = ["Off", "Neutral", "AgX"], default = 1)]
    pub highlights: u32,

    /// How much work each pixel does: Draft trims steps, orders and
    /// cells for a quick preview, High adds more for a final export.
    #[choice(options = ["Draft", "Normal", "High"], default = 1)]
    pub quality: u32,

    /// Per instance, for the speckle, the wobble and the wear.
    #[seed]
    pub seed: u32,

    /// The host-uniform Mix every effect ends with, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// One frame of Hologram: the kernel's uniform rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HologramParams {
    /// The rows: Holographic foil's layout with the replay's own in it, as
    /// `pack.rs` says.
    pub blocks: [[f32; 4]; BLOCKS],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl HologramParams {
    /// Mix 0: the picture goes through untouched.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.mix <= 0.0
    }
}

impl Hologram {
    /// Layer time, seconds. Never a panel row.
    pub const DERIVED_TIME: ParamId = ParamId::new("derived.time");

    /// The numbers the kernel reads, from the rows in `p`, on a `w` by `h`
    /// raster.
    #[must_use]
    pub fn packed(self, p: Params<'_>, w: u32, h: u32) -> HologramParams {
        let time = f64::from(p.float(Self::DERIVED_TIME, 0.0));
        let foil = pack::foil_of(&self);
        let m = motion::read(&foil, p, time);
        HologramParams {
            blocks: pack::blocks(&self, &foil, w, h, &m),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Hologram's behaviour.
pub struct HologramDef;

impl EffectDef for HologramDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Hologram as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [EffectPreset] {
        presets::HOLOGRAM_LOOKS
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        motion::resolve(cx, push);
    }

    fn moves_with_time(&self, inst: &EffectInstance) -> bool {
        motion::moves_with_time(inst)
    }
}
