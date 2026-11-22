//! Holographic foil: the layer becomes a sheet of embossed rainbow foil that
//! tips under a set of lights, and its colours sweep as it moves.
//!
//! The foil is modelled as it is made: a fine grating pressed into aluminium
//! under a clear lacquer. Each light splits into a rainbow at the angle the
//! grating equation gives, the flat part mirrors the lights and a soft grey
//! studio, and the lacquer adds its own gloss. The layer's alpha is the
//! sheet's outline, and its colour is printed on the foil as Print says.
//!
//! A fine relief bends the surface: built-in scratches and other textures,
//! all sized in sheet millimetres, or a Height map layer. Further layer rows
//! mask the foil under a print, steer the grooves, set their spacing and say
//! what the foil mirrors.
//!
//! The sheet has a real width and is seen from a real distance, so the
//! colours change across it as they would on a card held in the hand. The
//! output is scene-linear light, brighter than white in the glints, ready for
//! Glow.
//!
//! GPU only: `apply_cpu` keeps its identity default.
//!
//! The model, its constants and its controls follow holospot by Dmitry Kurash
//! (MIT, Copyright (c) 2026 Dmitry Kurash), reimplemented here. The colour
//! uses Wyman, Sloan and Shirley's fit of the CIE observer (JCGT 2013); the
//! highlights use the Khronos PBR Neutral curve (Apache 2.0) and AgX as
//! three.js has it (MIT). The kernels also carry Brian Karis's environment
//! BRDF fit, Sergei Winitzki's inverse error function, Inigo Quilez's hexagon
//! and Voronoi edges (MIT) and Jarzynski and Olano's pcg3d hash, each named
//! where it is used.

use crate::fx::{
    builtins::MAP_PLACEMENT_OPTIONS, params::ParamId, params::Params, params::Value,
    presets::EffectPreset, registry::EffectDef, registry::EffectMetadata, registry::ResolveCx,
    schema::EffectSchema,
};
use crate::model::EffectInstance;
use lumit_fx_macros::Effect;

mod eta;
mod groups;
mod lights;
mod memo;
pub(crate) mod motion;
pub(crate) mod pack;
mod presets;
mod spectra;
mod surface;
mod table;

/// The rows of four floats the kernel reads.
pub const BLOCKS: usize = table::ROWS;

const SPECTRUM_OPTIONS: &[&str] = &["Equal", "Daylight", "Tungsten", "LED", "Fluorescent"];

/// The Material choice that draws cracked shards.
pub(super) const SHATTER: u32 = 1;

/// The Material choice that draws flakes in a binder.
pub(super) const GLITTER: u32 = 2;

/// The Material choice that draws dots on matte metal.
pub(super) const SPARKLE: u32 = 3;

/// The Material choice that draws a vortex.
pub(super) const VORTEX: u32 = 4;

/// Holographic foil's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "holographic_foil",
    label = "Holographic foil",
    version = 1,
    category = Stylise,
    // Up to 18 lamps (three spots, each twice under thin marks, and up to
    // twelve panel cells), three orders each, and the studio, a pixel.
    cost = Heavy,
    roi = Exact,
    // Not seeded: layer time joins the frame's key only while the sheet
    // moves with it (`moves_with_time`).
    premultiplied = true,
    groups = groups::HOLOGRAPHIC_FOIL_GROUPS,
    enabled_when = groups::HOLOGRAPHIC_FOIL_ENABLED_WHEN,
    matte = (
        "matte",
        "scales Coverage per pixel: white keeps the grating, black leaves plain mirror foil",
    ),
)]
pub struct HolographicFoil {
    /// What the foil is made of: one grating, cracked shards, flakes in a
    /// binder, dots on matte metal, or a spiral.
    #[choice(options = ["Prism", "Shatter", "Glitter", "Sparkle", "Vortex"], default = 0)]
    pub material: u32,

    /// What the layer's own colour does: ink over the foil, nothing, or an
    /// opaque print with foil where it is white.
    #[choice(options = ["Ink over foil", "Foil only", "Opaque print"], default = 0)]
    pub print: u32,

    /// Tips the top edge toward the viewer, degrees. Turned past edge-on,
    /// the sheet shows its plain back.
    #[slider(
        min = -60.0,
        max = 60.0,
        default = -20.0,
        hard_min = -89.0,
        hard_max = 89.0,
        unit = Degrees
    )]
    pub tilt_x: f32,

    /// Turns the face toward the right, degrees. Turned past edge-on, the
    /// sheet shows its plain back.
    #[slider(
        min = -60.0,
        max = 60.0,
        default = -25.0,
        hard_min = -89.0,
        hard_max = 89.0,
        unit = Degrees
    )]
    pub tilt_y: f32,

    /// Turns the grating within the sheet, degrees.
    #[slider(min = -180.0, max = 180.0, default = 0.0, unit = Degrees)]
    pub spin: f32,

    /// How far the sheet wobbles on its own, degrees.
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

    /// How much of each frame the colours blur over while the sheet or the
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

    /// How wide the sheet is in the world, millimetres.
    #[slider(
        label = "Sheet width (mm)",
        min = 20.0,
        max = 1000.0,
        default = 90.0,
        hard_min = 1.0,
        unit = Raw
    )]
    pub sheet_width: f32,

    /// How far the eye is from the sheet, metres. Nearer gives more bands.
    #[slider(
        label = "View distance (m)",
        min = 0.1,
        max = 5.0,
        default = 0.35,
        hard_min = 0.05,
        unit = Raw
    )]
    pub view_distance: f32,

    /// The groove spacing, nanometres. Wider packs the colours nearer the
    /// glint.
    #[slider(
        label = "Pitch (nm)",
        min = 500.0,
        max = 2000.0,
        default = 1765.0,
        hard_min = 200.0,
        hard_max = 5000.0,
        unit = Raw
    )]
    pub pitch: f32,

    /// Which way the grooves run, degrees from the sheet's x axis.
    #[slider(min = 0.0, max = 180.0, default = 120.0, unit = Degrees)]
    pub groove_angle: f32,

    /// Straight grooves, rings like a compact disc, a sunburst, or grooves
    /// the Direction map steers (rings until it names a layer).
    #[choice(options = ["Linear", "Rings", "Sunburst", "From map"], default = 1)]
    pub pattern: u32,

    /// Where rings and a sunburst are centred, per cent of the width.
    #[slider(
        label = "Centre X",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = -100.0,
        hard_max = 200.0,
        unit = Percent
    )]
    pub centre_x: f32,

    /// Where rings and a sunburst are centred, per cent of the height.
    #[slider(
        label = "Centre Y",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = -100.0,
        hard_max = 200.0,
        unit = Percent
    )]
    pub centre_y: f32,

    /// How deep the grooves are, nanometres. Deeper moves light from the
    /// mirror into the rainbow.
    #[slider(
        label = "Depth (nm)",
        min = 0.0,
        max = 200.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Raw
    )]
    pub depth: f32,

    /// How much of the sheet carries the grating. The rest is plain mirror.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub coverage: f32,

    /// How far each band spreads along the grooves, degrees.
    #[slider(min = 0.0, max = 30.0, default = 7.0, hard_min = 0.0, hard_max = 60.0, unit = Degrees)]
    pub fan: f32,

    /// How rough the metal is, degrees. Softens the bands and the glints.
    #[slider(min = 0.0, max = 3.0, default = 0.05, hard_min = 0.0, hard_max = 20.0, unit = Degrees)]
    pub softness: f32,

    /// How far the grooves stray from their direction, degrees.
    #[slider(min = 0.0, max = 5.0, default = 0.3, hard_min = 0.0, hard_max = 30.0, unit = Degrees)]
    pub orientation_jitter: f32,

    /// How far the groove spacing strays, per cent. Softens the hues.
    #[slider(min = 0.0, max = 3.0, default = 0.5, hard_min = 0.0, hard_max = 20.0, unit = Percent)]
    pub pitch_jitter: f32,

    /// The refractive index of the lacquer over the grooves.
    #[slider(
        label = "Lacquer index",
        min = 1.4,
        max = 1.7,
        default = 1.56,
        hard_min = 1.3,
        hard_max = 2.0,
        unit = Raw
    )]
    pub lacquer: f32,

    /// How far out a vortex's grooves turn to 45 degrees, millimetres. 0
    /// gives plain rings.
    #[slider(
        label = "Swirl (mm)",
        min = 0.0,
        max = 60.0,
        default = 15.0,
        hard_min = 0.0,
        hard_max = 1000.0,
        unit = Raw
    )]
    pub swirl: f32,

    /// Which way the vortex turns.
    #[choice(label = "Turn", options = ["Anticlockwise", "Clockwise"], default = 0)]
    pub handedness: u32,

    /// How many spiral bands of plainer foil cross the vortex.
    #[counter(min = 0, max = 96, default = 24, hard_min = 0, hard_max = 400, unit = Raw)]
    pub bands: i32,

    /// How much plainer the bands are, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 45.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub band_contrast: f32,

    /// The mean size of a shard, millimetres on the sheet.
    #[slider(
        label = "Shard size (mm)",
        min = 0.5,
        max = 8.0,
        default = 2.5,
        hard_min = 0.05,
        hard_max = 100.0,
        unit = Raw
    )]
    pub shard_size: f32,

    /// How irregular the cracks are. 0 is a square grid.
    #[slider(min = 0.0, max = 1.0, default = 0.9, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub irregularity: f32,

    /// How many straight cuts split each piece on average, for slivers and
    /// T-shaped joins.
    #[slider(min = 0.0, max = 3.0, default = 1.8, hard_min = 0.0, hard_max = 3.0, unit = Raw)]
    pub splits: f32,

    /// How many groove directions the shards choose from. 0 is any.
    #[counter(min = 0, max = 24, default = 0, hard_min = 0, hard_max = 180, unit = Raw)]
    pub directions: i32,

    /// How far each shard tips under the lacquer, degrees, so neighbours
    /// flash at different tilts.
    #[slider(label = "Shard tilt", min = 0.0, max = 6.0, default = 1.0, hard_min = 0.0, hard_max = 30.0, unit = Degrees)]
    pub shard_tilt: f32,

    /// The groove spacing of the first family of shards, nanometres.
    #[slider(label = "Pitch A (nm)", min = 500.0, max = 2000.0, default = 1150.0, hard_min = 200.0, hard_max = 5000.0, unit = Raw)]
    pub pitch_a: f32,

    /// The groove spacing of the second family of shards, nanometres.
    #[slider(label = "Pitch B (nm)", min = 500.0, max = 2000.0, default = 1080.0, hard_min = 200.0, hard_max = 5000.0, unit = Raw)]
    pub pitch_b: f32,

    /// The groove spacing of the third family of shards, nanometres.
    #[slider(label = "Pitch C (nm)", min = 500.0, max = 2000.0, default = 870.0, hard_min = 200.0, hard_max = 5000.0, unit = Raw)]
    pub pitch_c: f32,

    /// How much the groove spacing changes across a shard, per cent.
    #[slider(min = 0.0, max = 5.0, default = 0.0, hard_min = 0.0, hard_max = 50.0, unit = Percent)]
    pub chirp: f32,

    /// How high the creases along the cracks stand, micrometres. Below 0
    /// they are grooves.
    #[slider(label = "Crease height (µm)", min = -0.4, max = 0.4, default = 0.3, hard_min = -5.0, hard_max = 5.0, unit = Raw)]
    pub ridge_height: f32,

    /// How far a crease reaches either side of its crack, micrometres.
    #[slider(label = "Crease width (µm)", min = 10.0, max = 200.0, default = 40.0, hard_min = 1.0, hard_max = 2000.0, unit = Raw)]
    pub ridge_width: f32,

    /// How many of the cracks carry a crease, per cent.
    #[slider(min = 0.0, max = 100.0, default = 50.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub creased: f32,

    /// How wide a glitter flake is, flat to flat, micrometres.
    #[slider(label = "Flake size (µm)", min = 50.0, max = 400.0, default = 200.0, hard_min = 10.0, hard_max = 5000.0, unit = Raw)]
    pub flake_size: f32,

    /// The outline of a flake.
    #[choice(label = "Flake shape", options = ["Hex", "Square", "Round"], default = 0)]
    pub flake_shape: u32,

    /// How many layers of flakes lie over each other.
    #[counter(label = "Flake layers", min = 1, max = 3, default = 2, hard_min = 1, hard_max = 3, unit = Raw)]
    pub flake_layers: i32,

    /// How far the flakes tip, degrees: how far from the mirror angle they
    /// still flash.
    #[slider(label = "Flake tilt", min = 0.0, max = 20.0, default = 4.6, hard_min = 0.0, hard_max = 45.0, unit = Degrees)]
    pub flake_tilt: f32,

    /// How much a flake bends within itself, degrees. More gives bigger,
    /// dimmer glints.
    #[slider(label = "Flake curve", min = 0.05, max = 3.0, default = 0.4, hard_min = 0.0, hard_max = 20.0, unit = Degrees)]
    pub flake_curve: f32,

    /// The finest groove spacing a flake can have, nanometres.
    #[slider(label = "Finest pitch (nm)", min = 400.0, max = 2000.0, default = 665.0, hard_min = 200.0, hard_max = 5000.0, unit = Raw)]
    pub pitch_min: f32,

    /// The widest groove spacing a flake can have, nanometres.
    #[slider(label = "Widest pitch (nm)", min = 400.0, max = 2000.0, default = 740.0, hard_min = 200.0, hard_max = 5000.0, unit = Raw)]
    pub pitch_max: f32,

    /// How wide a star is on average, micrometres. Stars are larger dots with
    /// round grooves.
    #[slider(label = "Star size (µm)", min = 150.0, max = 1500.0, default = 600.0, hard_min = 20.0, hard_max = 20000.0, unit = Raw)]
    pub star_size: f32,

    /// How many of the places for a star hold one, per cent.
    #[slider(label = "Stars", min = 0.0, max = 30.0, default = 3.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub star_density: f32,

    /// The ink under the flakes, seen in the gaps.
    #[colour(label = "Base colour", default = [0.0037, 0.003, 0.0176, 1.0], max = 4.0)]
    pub base_colour: [f32; 4],

    /// How wide a sparkle dot is on average, micrometres.
    #[slider(label = "Dot size (µm)", min = 100.0, max = 1500.0, default = 470.0, hard_min = 10.0, hard_max = 20000.0, unit = Raw)]
    pub dot_size: f32,

    /// How much of the sheet the dots cover, per cent.
    #[slider(label = "Dot cover", min = 10.0, max = 80.0, default = 40.0, hard_min = 1.0, hard_max = 90.0, unit = Percent)]
    pub dot_cover: f32,

    /// How lobed and uneven the dots are. 0 is round and even.
    #[slider(label = "Irregularity", min = 0.0, max = 1.0, default = 0.8, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub dot_irregularity: f32,

    /// How far each dot tips, degrees.
    #[slider(label = "Dot tilt", min = 0.0, max = 5.0, default = 0.3, hard_min = 0.0, hard_max = 30.0, unit = Degrees)]
    pub dot_tilt: f32,

    /// How much a dot bulges within itself, degrees.
    #[slider(label = "Dome", min = 0.05, max = 2.0, default = 0.3, hard_min = 0.0, hard_max = 20.0, unit = Degrees)]
    pub dot_dome: f32,

    /// How many of the dots are dyed satin specks, per cent.
    #[slider(label = "Specks", min = 0.0, max = 30.0, default = 9.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub speck_share: f32,

    /// Where the specks' dye lets most light through, nanometres.
    #[slider(label = "Speck colour (nm)", min = 420.0, max = 520.0, default = 460.0, hard_min = 360.0, hard_max = 830.0, unit = Raw)]
    pub speck_peak: f32,

    /// How wide the specks' dye lets light through, nanometres.
    #[slider(label = "Speck width (nm)", min = 10.0, max = 60.0, default = 25.0, hard_min = 1.0, hard_max = 300.0, unit = Raw)]
    pub speck_width: f32,

    /// How much the specks' dye lets through away from its colour, per cent.
    #[slider(label = "Speck floor", min = 0.0, max = 100.0, default = 30.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub speck_floor: f32,

    /// How matte the metal between the dots is, degrees of slope.
    #[slider(label = "Base roughness", min = 1.0, max = 20.0, default = 8.0, hard_min = 0.0, hard_max = 45.0, unit = Degrees)]
    pub base_matte: f32,

    /// How big the grains of the matte metal are, as a share of the dot size.
    #[slider(label = "Sand size", min = 0.15, max = 1.0, default = 0.5, hard_min = 0.02, hard_max = 10.0, unit = Raw)]
    pub sand_size: f32,

    /// How far the grains of the matte metal tip, degrees. Never more than
    /// Base roughness.
    #[slider(label = "Sand tilt", min = 0.0, max = 8.0, default = 5.0, hard_min = 0.0, hard_max = 45.0, unit = Degrees)]
    pub sand_tilt: f32,

    /// A dye in the lacquer, or Auto: the dye that suits the Material.
    #[choice(
        options = ["Clear", "Gold", "Blue", "Custom", "Auto"],
        dividers_after = &[2, 3],
        default = 4
    )]
    pub tint: u32,

    /// Which side of its edge a custom dye lets through.
    #[choice(label = "Passes", options = ["Long waves", "Short waves"], default = 0)]
    pub tint_pass: u32,

    /// Where a custom dye turns, nanometres.
    #[slider(
        label = "Edge (nm)",
        min = 400.0,
        max = 700.0,
        default = 505.0,
        hard_min = 360.0,
        hard_max = 830.0,
        unit = Raw
    )]
    pub tint_edge: f32,

    /// How gradually a custom dye turns, nanometres.
    #[slider(
        label = "Edge width (nm)",
        min = 5.0,
        max = 150.0,
        default = 35.0,
        hard_min = 1.0,
        hard_max = 300.0,
        unit = Raw
    )]
    pub tint_width: f32,

    /// How much a custom dye lets through on its blocked side, per cent.
    #[slider(
        label = "Floor",
        min = 0.0,
        max = 100.0,
        default = 6.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub tint_floor: f32,

    /// A ready rig, Custom to set each light, or Auto: the rig that suits
    /// the Material.
    #[choice(
        options = [
            "Studio", "Spot", "Softbox", "Strip", "Dark", "Desk", "Office", "Raking", "Custom",
            "Auto"
        ],
        dividers_after = &[7, 8],
        default = 9
    )]
    pub lighting: u32,

    /// Turns every light and the studio about the view axis, degrees.
    #[slider(min = -180.0, max = 180.0, default = 0.0, unit = Degrees)]
    pub rig_rotation: f32,

    /// Spot A lights the sheet.
    #[toggle(label = "On", default = true)]
    pub spot_a_on: bool,

    /// Where spot A is, as seen from the sheet: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.38, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_a_x: f32,

    /// Where spot A is, as seen from the sheet: -1 to 1 up.
    #[slider(label = "Direction y", min = -1.0, max = 1.0, default = 0.57, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_a_y: f32,

    /// How wide spot A looks from the sheet, degrees.
    #[slider(label = "Size", min = 0.2, max = 10.0, default = 1.5, hard_min = 0.05, hard_max = 45.0, unit = Degrees)]
    pub spot_a_size: f32,

    /// How bright spot A is.
    #[slider(label = "Intensity", min = 0.0, max = 5000.0, default = 970.0, hard_min = 0.0, unit = Raw)]
    pub spot_a_intensity: f32,

    /// How far spot A is from the sheet, metres.
    #[slider(label = "Distance (m)", min = 0.3, max = 3.0, default = 1.0, hard_min = 0.05, unit = Raw)]
    pub spot_a_distance: f32,

    /// The colour of spot A's light.
    #[choice(label = "Spectrum", options = *SPECTRUM_OPTIONS, default = 0)]
    pub spot_a_spectrum: u32,

    /// Spot B lights the sheet.
    #[toggle(label = "On", default = true)]
    pub spot_b_on: bool,

    /// Where spot B is, as seen from the sheet: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.12, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_b_x: f32,

    /// Where spot B is, as seen from the sheet: -1 to 1 up.
    #[slider(label = "Direction y", min = -1.0, max = 1.0, default = -0.17, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_b_y: f32,

    /// How wide spot B looks from the sheet, degrees.
    #[slider(label = "Size", min = 0.2, max = 10.0, default = 2.5, hard_min = 0.05, hard_max = 45.0, unit = Degrees)]
    pub spot_b_size: f32,

    /// How bright spot B is.
    #[slider(label = "Intensity", min = 0.0, max = 5000.0, default = 485.0, hard_min = 0.0, unit = Raw)]
    pub spot_b_intensity: f32,

    /// How far spot B is from the sheet, metres.
    #[slider(label = "Distance (m)", min = 0.3, max = 3.0, default = 1.2, hard_min = 0.05, unit = Raw)]
    pub spot_b_distance: f32,

    /// The colour of spot B's light.
    #[choice(label = "Spectrum", options = *SPECTRUM_OPTIONS, default = 0)]
    pub spot_b_spectrum: u32,

    /// Spot C lights the sheet.
    #[toggle(label = "On", default = true)]
    pub spot_c_on: bool,

    /// Where spot C is, as seen from the sheet: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.42, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_c_x: f32,

    /// Where spot C is, as seen from the sheet: -1 to 1 up.
    #[slider(label = "Direction y", min = -1.0, max = 1.0, default = 0.71, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub spot_c_y: f32,

    /// How wide spot C looks from the sheet, degrees.
    #[slider(label = "Size", min = 0.2, max = 10.0, default = 1.0, hard_min = 0.05, hard_max = 45.0, unit = Degrees)]
    pub spot_c_size: f32,

    /// How bright spot C is.
    #[slider(label = "Intensity", min = 0.0, max = 5000.0, default = 970.0, hard_min = 0.0, unit = Raw)]
    pub spot_c_intensity: f32,

    /// How far spot C is from the sheet, metres.
    #[slider(label = "Distance (m)", min = 0.3, max = 3.0, default = 1.0, hard_min = 0.05, unit = Raw)]
    pub spot_c_distance: f32,

    /// The colour of spot C's light.
    #[choice(label = "Spectrum", options = *SPECTRUM_OPTIONS, default = 0)]
    pub spot_c_spectrum: u32,

    /// The panel lights the sheet.
    #[toggle(label = "On", default = true)]
    pub panel_on: bool,

    /// Where the panel is, as seen from the sheet: -1 to 1 across.
    #[slider(label = "Direction x", min = -1.0, max = 1.0, default = -0.43, hard_min = -1.0, hard_max = 1.0, unit = Raw)]
    pub panel_x: f32,

    /// Where the panel is, as seen from the sheet: -1 to 1 up.
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

    /// How far the panel is from the sheet, metres.
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

    /// A pale sheet just above the viewer, so bare foil reads grey.
    #[slider(min = 0.0, max = 2.0, default = 0.2, hard_min = 0.0, unit = Raw)]
    pub scrim: f32,

    /// The faint light everywhere else in front.
    #[slider(min = 0.0, max = 0.5, default = 0.04, hard_min = 0.0, unit = Raw)]
    pub room: f32,

    /// How rough the clear coat is: low is a sharp gloss.
    #[slider(min = 0.01, max = 0.5, default = 0.01, hard_min = 0.005, hard_max = 1.0, unit = Raw)]
    pub coat_roughness: f32,

    /// The fine relief pressed into the surface, or a bevel the sheet's own
    /// outline presses into it. A Height map layer takes its place.
    #[choice(
        label = "Relief",
        options = [
            "Scratches", "Brushed", "Hammered", "Crinkled", "Linen", "Fingerprint", "Sandblast",
            "None", "Bevel from alpha"
        ],
        dividers_after = &[6, 7],
        default = 0
    )]
    pub relief_source: u32,

    /// How strongly the relief bends the surface.
    #[slider(label = "Strength", min = 0.0, max = 3.0, default = 0.78, hard_min = 0.0, hard_max = 10.0, unit = Raw)]
    pub relief: f32,

    /// How many times the relief repeats across the sheet. Higher is finer.
    #[slider(label = "Scale", min = 1.0, max = 12.0, default = 3.5, hard_min = 0.1, hard_max = 100.0, unit = Raw)]
    pub relief_scale: f32,

    /// Which layer the relief bends: the foil under the lacquer, or only the
    /// varnish on top.
    #[choice(label = "Relief on", options = ["Foil", "Varnish"], default = 0)]
    pub relief_on: u32,

    /// Turns grooves into ridges.
    #[toggle(label = "Invert", default = true)]
    pub relief_invert: bool,

    /// How far in from the outline the bevel's wall reaches, millimetres.
    #[slider(label = "Bevel width (mm)", min = 0.1, max = 5.0, default = 1.5, hard_min = 0.02, hard_max = 50.0, unit = Raw)]
    pub bevel_width: f32,

    /// How high the bevel stands, micrometres.
    #[slider(label = "Bevel height (µm)", min = 0.0, max = 500.0, default = 150.0, hard_min = 0.0, hard_max = 5000.0, unit = Raw)]
    pub bevel_height: f32,

    /// The wall's shape: rounded, a flat slope, a cushion that rises from
    /// the edge, or pressed in.
    #[choice(label = "Profile", options = ["Round", "Bevel", "Dome", "Deboss"], default = 0)]
    pub bevel_profile: u32,

    /// How much the outline is rounded before the wall is shaped, per cent.
    #[slider(label = "Softness", min = 0.0, max = 100.0, default = 20.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub bevel_softness: f32,

    /// How far broad waves in the sheet tip it, degrees.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 10.0, unit = Degrees)]
    pub waviness: f32,

    /// How rough the varnish's fine ripple is, micrometres.
    #[slider(label = "Orange peel (µm)", min = 0.0, max = 2.0, default = 0.0, hard_min = 0.0, hard_max = 20.0, unit = Raw)]
    pub orange_peel: f32,

    /// Fine lines embossed across the sheet, on the layer Relief is on.
    #[choice(options = ["None", "Lines", "Crosshatch"], default = 0)]
    pub lines: u32,

    /// The spacing of the embossed lines, millimetres.
    #[slider(label = "Line pitch (mm)", min = 0.15, max = 2.0, default = 1.17, hard_min = 0.01, hard_max = 20.0, unit = Raw)]
    pub line_pitch: f32,

    /// How deep the embossed lines are, micrometres.
    #[slider(label = "Line height (µm)", min = 0.0, max = 15.0, default = 0.5, hard_min = 0.0, hard_max = 100.0, unit = Raw)]
    pub line_height: f32,

    /// How far the sheet bows in its middle, millimetres.
    #[slider(label = "Bow (mm)", min = 0.0, max = 2.0, default = 1.6, hard_min = -20.0, hard_max = 20.0, unit = Raw)]
    pub bow: f32,

    /// Whether the sheet bows across its width or along its height.
    #[choice(label = "Bow axis", options = ["Across", "Along"], default = 0)]
    pub bow_axis: u32,

    /// Handling marks: smudges, hairline scratches round the highlights, dust
    /// and patchy foil, per cent.
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub wear: f32,

    /// A layer whose brightness is the surface's height. Decided by the
    /// render, so always `false` here.
    #[layer(label = "Height map", self_default = false)]
    pub height_map: bool,

    /// Where the Height map layer's picture sits on this one.
    #[choice(label = "Placement", options = *MAP_PLACEMENT_OPTIONS, default = 1)]
    pub height_map_placement: u32,

    /// How many times the Height map repeats across the sheet.
    #[slider(label = "Repeats", min = 1.0, max = 12.0, default = 1.0, hard_min = 1.0, hard_max = 100.0, unit = Raw)]
    pub height_map_scale: f32,

    /// How the Height map meets itself where it repeats: as it is, mirrored
    /// so a picture that does not tile has no seam, or held at its edges.
    #[choice(label = "Wrap", options = ["Repeat", "Mirror", "Clamp"], default = 0)]
    pub height_map_wrap: u32,

    /// A layer whose white shows bare foil through an opaque print. Decided
    /// by the render, so always `false` here.
    #[layer(label = "Foil mask", self_default = false)]
    pub foil_mask: bool,

    /// Where the Foil mask layer's picture sits on this one.
    #[choice(label = "Placement", options = *MAP_PLACEMENT_OPTIONS, default = 0)]
    pub foil_mask_placement: u32,

    /// A layer that steers the grooves. Decided by the render, so always
    /// `false` here.
    #[layer(label = "Direction map", self_default = false)]
    pub direction_map: bool,

    /// Where the Direction map layer's picture sits on this one.
    #[choice(label = "Placement", options = *MAP_PLACEMENT_OPTIONS, default = 0)]
    pub direction_map_placement: u32,

    /// How the Direction map is read: its brightness as an angle, or grooves
    /// that run along its edges.
    #[choice(label = "Reading", options = ["Angle", "Along edges"], default = 1)]
    pub direction_mode: u32,

    /// A layer that sets the groove spacing. Decided by the render, so
    /// always `false` here.
    #[layer(label = "Pitch map", self_default = false)]
    pub pitch_map: bool,

    /// Where the Pitch map layer's picture sits on this one.
    #[choice(label = "Placement", options = *MAP_PLACEMENT_OPTIONS, default = 0)]
    pub pitch_map_placement: u32,

    /// The groove spacing where the Pitch map is white, nanometres. Black
    /// keeps Pitch.
    #[slider(
        label = "Pitch at white (nm)",
        min = 500.0,
        max = 2000.0,
        default = 1600.0,
        hard_min = 200.0,
        hard_max = 5000.0,
        unit = Raw
    )]
    pub pitch_white: f32,

    /// A layer the foil mirrors. Decided by the render, so always `false`
    /// here.
    #[layer(label = "Environment", self_default = false)]
    pub environment: bool,

    /// How much of what the foil mirrors is the Environment layer. At 100 it
    /// takes the studio's place; above, it is brighter.
    #[slider(label = "Amount", min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, hard_max = 1000.0, unit = Percent)]
    pub environment_amount: f32,

    /// A gain on the light, stops.
    #[slider(min = -3.0, max = 3.0, default = 0.0, unit = Raw)]
    pub exposure: f32,

    /// How light above white is rolled off. Off keeps it all, for Glow.
    #[choice(options = ["Off", "Neutral", "AgX"], default = 1)]
    pub highlights: u32,

    /// How much work each pixel does: Draft trims orders, cells and
    /// candidates for a quick preview, High adds more for a final export.
    #[choice(options = ["Draft", "Normal", "High"], default = 1)]
    pub quality: u32,

    /// Per instance, for the layouts that scatter.
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

/// One frame of Holographic foil: the kernel's uniform rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HolographicFoilParams {
    /// The rows, laid out as the table in `pack.rs` says.
    pub blocks: [[f32; 4]; BLOCKS],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl HolographicFoilParams {
    /// Mix 0: the picture goes through untouched.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.mix <= 0.0
    }
}

impl HolographicFoil {
    /// Layer time, seconds. Never a panel row.
    pub const DERIVED_TIME: ParamId = ParamId::new("derived.time");

    /// The numbers the kernel reads, from the rows in `p`, on a `w` by `h`
    /// raster. The Lighting choice's rig replaces the light rows unless it is
    /// Custom.
    #[must_use]
    pub fn packed(self, p: Params<'_>, w: u32, h: u32) -> HolographicFoilParams {
        let time = f64::from(p.float(Self::DERIVED_TIME, 0.0));
        let motion = motion::read(&self, p, time);
        HolographicFoilParams {
            blocks: pack::blocks(&self, w, h, &motion),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Holographic foil's behaviour.
pub struct HolographicFoilDef;

impl EffectDef for HolographicFoilDef {
    fn schema(&self) -> &'static EffectSchema {
        &<HolographicFoil as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [EffectPreset] {
        presets::HOLOGRAPHIC_FOIL_LOOKS
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        motion::resolve(cx, push);
    }

    fn moves_with_time(&self, inst: &EffectInstance) -> bool {
        motion::moves_with_time(inst)
    }
}
