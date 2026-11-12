//! Correction: one whole grade node. Wheels, log wheels, adjustments, an RGB
//! mixer, curves and hue curves, keyed by a qualifier, four windows and the
//! Matte.
//!
//! **In plain terms.** Everything a colourist reaches for on one node of a
//! grade, as one effect. The grade is worked in its Space, and the key says
//! where it lands: white grades, black leaves the picture as it was. Nothing
//! moves until a row does, so a fresh Correction hands the picture on
//! untouched. The maths is written out in [`maths`].
//!
//! Combine says how the node meets the one above it in the stack. The stack
//! reads it ([`combine`]); this effect draws every node as Serial.
//!
//! Row ids are fixed: the Grade panel and the solvers write them by name. The
//! HDR twirl's rows are `hdr_*`, listed in [`hdr`]. The 3D qualifier (Qualifier
//! 4) keys on `qual_3d_keep`, `qual_3d_keep_luma`, `qual_3d_drop`,
//! `qual_3d_drop_luma`, `qual_3d_size` and `qual_3d_soft`, how they are stored
//! written out in [`maths`].

pub mod combine;
pub mod cpu;
pub mod groups;
pub mod hdr;
pub mod maths;
pub mod packed;

use crate::fx::{
    params::CurvePoints,
    params::{ParamId, Params, Value},
    registry::{EffectDef, EffectMetadata, ResolveCx},
    schema::EffectSchema,
    shape_points::ShapePoints,
};
use lumit_fx_macros::Effect;

use packed::HUE_FLAT;

/// Where the grade is worked.
pub const SPACE_OPTIONS: [&str; 3] = ["Video", "Log", "Linear"];

/// How a node meets the one above it.
pub const COMBINE_OPTIONS: [&str; 3] = ["Serial", "Parallel", "Layer"];

/// What the qualifier keys on.
pub const QUALIFIER_OPTIONS: [&str; 5] = ["Off", "HSL", "RGB", "Luma", "3D"];

/// A window's shape.
pub const WINDOW_OPTIONS: [&str; 5] = ["Off", "Ellipse", "Rectangle", "Polygon", "Gradient"];

/// Correction's controls. The ids are fixed: the Grade panel and the solvers
/// write them by name.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "correction",
    label = "Correction",
    version = 1,
    category = Colour,
    cost = Moderate,
    // The qualifier's Blur, Grow and Denoise reach this far at most.
    roi = PaddedPx(128.0),
    // A grade shifts matte edges when worked premultiplied.
    premultiplied = false,
    groups = groups::CORRECTION_GROUPS,
    enabled_when = groups::CORRECTION_ENABLED_WHEN,
    matte = (
        "matte",
        "joins the node's key: white grades, black leaves the picture alone",
    ),
)]
pub struct Correction {
    /// Where the grade is worked: gamma 2.4, ACEScct, or scene linear.
    #[choice(options = SPACE_OPTIONS, default = 0)]
    pub space: u32,
    /// How this node meets the one above it. The stack reads it.
    #[choice(options = COMBINE_OPTIONS, default = 0)]
    pub combine: u32,

    /// Raises or crushes the blacks, leaving white where it is.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub lift: [f32; 4],
    /// Bends the middle; positive brightens.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub gamma: [f32; 4],
    /// Scales every value.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], min = 0.0, max = 4.0)]
    pub gain: [f32; 4],
    /// Adds to every value, before the other three.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub offset: [f32; 4],

    /// Added below Low range.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub shadow: [f32; 4],
    /// Added between the two ranges.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub midtone: [f32; 4],
    /// Added above High range.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub highlight: [f32; 4],
    /// Where shadows end, in the grading space.
    #[bounded(min = 0.0, max = 1.0, default = 0.333, unit = Raw)]
    pub low_range: f32,
    /// Where highlights start, in the grading space.
    #[bounded(min = 0.0, max = 1.0, default = 0.55, unit = Raw)]
    pub high_range: f32,

    /// Stops, in scene linear before the grade.
    #[slider(min = -5.0, max = 5.0, default = 0.0, hard_min = -20.0, hard_max = 20.0, unit = Raw)]
    pub exposure: f32,
    /// Temperature's own warm and cool.
    #[slider(
        min = -150.0,
        max = 150.0,
        default = 0.0,
        hard_min = -200.0,
        hard_max = 200.0,
        unit = Raw
    )]
    pub temperature: f32,
    /// Temperature's own green and magenta.
    #[slider(
        min = -150.0,
        max = 150.0,
        default = 0.0,
        hard_min = -200.0,
        hard_max = 200.0,
        unit = Raw
    )]
    pub tint: f32,
    /// Spreads values away from Pivot.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub contrast: f32,
    /// The value Contrast turns about.
    #[bounded(min = 0.0, max = 1.0, default = 0.435, unit = Raw)]
    pub pivot: f32,
    /// Colourfulness about luma.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub saturation: f32,
    /// Turns every hue, keeping luma.
    #[dial(default = 0.0, step = 15.0)]
    pub hue: f32,
    /// How much the wheels may move luma.
    #[bounded(label = "Lum mix", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub lum_mix: f32,
    /// Saturation that favours the quieter colours.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub colour_boost: f32,
    /// Lifts or lowers the darks.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub shadows: f32,
    /// Lifts or lowers the brights.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub highlights: f32,
    /// Local contrast in the midtones.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub midtone_detail: f32,

    /// The colour of the whole picture, stops a channel, luma kept.
    #[colour(label = "Global balance", default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub hdr_balance: [f32; 4],
    /// Lifts or sinks the darkest level, easing into the rest, scene linear.
    #[slider(
        label = "Black offset",
        min = -0.05,
        max = 0.05,
        default = 0.0,
        hard_min = -0.1,
        hard_max = 0.1,
        unit = Raw
    )]
    pub hdr_black_offset: f32,
    /// How softly every zone fades in from its range, stops.
    #[bounded(label = "Falloff", min = 0.0, max = 4.0, default = 2.0, unit = Raw)]
    pub hdr_falloff: f32,
    /// The colour of the deepest blacks, stops a channel, luma kept.
    #[colour(label = "Black balance", default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub hdr_black_balance: [f32; 4],
    /// Brightens or darkens the deepest blacks, stops.
    #[slider(
        label = "Black exposure",
        min = -4.0,
        max = 4.0,
        default = 0.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_black_exposure: f32,
    /// Colourfulness in the deepest blacks.
    #[slider(
        label = "Black saturation",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub hdr_black_saturation: f32,
    /// Where the zone starts, stops from grey; it reaches this and below.
    #[slider(
        label = "Black range",
        min = -8.0,
        max = 8.0,
        default = -3.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_black_range: f32,
    /// The colour of the darks, stops a channel, luma kept.
    #[colour(label = "Dark balance", default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub hdr_dark_balance: [f32; 4],
    /// Brightens or darkens the darks, stops.
    #[slider(
        label = "Dark exposure",
        min = -4.0,
        max = 4.0,
        default = 0.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_dark_exposure: f32,
    /// Colourfulness in the darks.
    #[slider(
        label = "Dark saturation",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub hdr_dark_saturation: f32,
    /// Where the zone starts, stops from grey; it reaches this and below.
    #[slider(
        label = "Dark range",
        min = -8.0,
        max = 8.0,
        default = -1.5,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_dark_range: f32,
    /// The colour of everything below grey, stops a channel, luma kept.
    #[colour(label = "Shadow balance", default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub hdr_shadow_balance: [f32; 4],
    /// Brightens or darkens everything below grey, stops.
    #[slider(
        label = "Shadow exposure",
        min = -4.0,
        max = 4.0,
        default = 0.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_shadow_exposure: f32,
    /// Colourfulness in everything below grey.
    #[slider(
        label = "Shadow saturation",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub hdr_shadow_saturation: f32,
    /// Where the zone starts, stops from grey; it reaches this and below.
    #[slider(
        label = "Shadow range",
        min = -8.0,
        max = 8.0,
        default = 1.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_shadow_range: f32,
    /// The colour of everything above grey, stops a channel, luma kept.
    #[colour(label = "Light balance", default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub hdr_light_balance: [f32; 4],
    /// Brightens or darkens everything above grey, stops.
    #[slider(
        label = "Light exposure",
        min = -4.0,
        max = 4.0,
        default = 0.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_light_exposure: f32,
    /// Colourfulness in everything above grey.
    #[slider(
        label = "Light saturation",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub hdr_light_saturation: f32,
    /// Where the zone starts, stops from grey; it reaches this and above.
    #[slider(
        label = "Light range",
        min = -8.0,
        max = 8.0,
        default = -1.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_light_range: f32,
    /// The colour of the brights, stops a channel, luma kept.
    #[colour(label = "Highlight balance", default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub hdr_highlight_balance: [f32; 4],
    /// Brightens or darkens the brights, stops.
    #[slider(
        label = "Highlight exposure",
        min = -4.0,
        max = 4.0,
        default = 0.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_highlight_exposure: f32,
    /// Colourfulness in the brights.
    #[slider(
        label = "Highlight saturation",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub hdr_highlight_saturation: f32,
    /// Where the zone starts, stops from grey; it reaches this and above.
    #[slider(
        label = "Highlight range",
        min = -8.0,
        max = 8.0,
        default = 1.5,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_highlight_range: f32,
    /// The colour of the brightest glints, stops a channel, luma kept.
    #[colour(label = "Specular balance", default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub hdr_specular_balance: [f32; 4],
    /// Brightens or darkens the brightest glints, stops.
    #[slider(
        label = "Specular exposure",
        min = -4.0,
        max = 4.0,
        default = 0.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_specular_exposure: f32,
    /// Colourfulness in the brightest glints.
    #[slider(
        label = "Specular saturation",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub hdr_specular_saturation: f32,
    /// Where the zone starts, stops from grey; it reaches this and above.
    #[slider(
        label = "Specular range",
        min = -8.0,
        max = 8.0,
        default = 4.0,
        hard_min = -16.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub hdr_specular_range: f32,

    /// What red is made of.
    #[colour(label = "Red output", default = [1.0, 0.0, 0.0, 1.0], min = -2.0, max = 2.0)]
    pub mixer_red: [f32; 4],
    /// What green is made of.
    #[colour(label = "Green output", default = [0.0, 1.0, 0.0, 1.0], min = -2.0, max = 2.0)]
    pub mixer_green: [f32; 4],
    /// What blue is made of.
    #[colour(label = "Blue output", default = [0.0, 0.0, 1.0, 1.0], min = -2.0, max = 2.0)]
    pub mixer_blue: [f32; 4],
    /// Puts luma back where the mixer found it.
    #[toggle(label = "Keep luma", default = false)]
    pub mixer_keep_luma: bool,

    /// Every channel's curve, after its own.
    #[curve(label = "Luma")]
    pub curve_y: CurvePoints,
    /// Red's own curve.
    #[curve(label = "Red")]
    pub curve_r: CurvePoints,
    /// Green's own curve.
    #[curve(label = "Green")]
    pub curve_g: CurvePoints,
    /// Blue's own curve.
    #[curve(label = "Blue")]
    pub curve_b: CurvePoints,
    /// Rolls the brights off before white.
    #[bounded(label = "High soft clip", min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub soft_high: f32,
    /// Rolls the darks off before black.
    #[bounded(label = "Low soft clip", min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub soft_low: f32,

    /// Turns hue by hue.
    #[curve(label = "Hue v hue", default = HUE_FLAT)]
    pub hue_hue: CurvePoints,
    /// Saturation by hue.
    #[curve(label = "Hue v sat", default = HUE_FLAT)]
    pub hue_sat: CurvePoints,
    /// Luma by hue.
    #[curve(label = "Hue v lum", default = HUE_FLAT)]
    pub hue_lum: CurvePoints,
    /// Saturation by luma.
    #[curve(label = "Lum v sat", default = HUE_FLAT)]
    pub lum_sat: CurvePoints,
    /// Saturation by saturation.
    #[curve(label = "Sat v sat", default = HUE_FLAT)]
    pub sat_sat: CurvePoints,
    /// Luma by saturation.
    #[curve(label = "Sat v lum", default = HUE_FLAT)]
    pub sat_lum: CurvePoints,

    /// What the qualifier keys on.
    #[choice(label = "Qualifier", options = QUALIFIER_OPTIONS, default = 0)]
    pub qual_mode: u32,
    /// Keys on hue.
    #[toggle(label = "Use hue", default = true)]
    pub qual_use_hue: bool,
    /// The hue keyed, from red.
    #[dial(label = "Hue", default = 0.0, step = 15.0)]
    pub qual_hue: f32,
    /// How wide a run of hues is keyed.
    #[bounded(label = "Hue width", min = 0.0, max = 360.0, default = 60.0, unit = Degrees)]
    pub qual_hue_width: f32,
    /// How far past the width the key fades.
    #[bounded(label = "Hue softness", min = 0.0, max = 180.0, default = 20.0, unit = Degrees)]
    pub qual_hue_soft: f32,
    /// Leans the width to one side of the centre.
    #[bounded(label = "Hue symmetry", min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub qual_hue_sym: f32,
    /// Keys on saturation.
    #[toggle(label = "Use saturation", default = true)]
    pub qual_use_sat: bool,
    /// The least saturation keyed.
    #[bounded(label = "Saturation low", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub qual_sat_low: f32,
    /// The most saturation keyed.
    #[bounded(
        label = "Saturation high",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        unit = Percent
    )]
    pub qual_sat_high: f32,
    /// How far past the range the key fades.
    #[bounded(
        label = "Saturation softness",
        min = 0.0,
        max = 100.0,
        default = 10.0,
        unit = Percent
    )]
    pub qual_sat_soft: f32,
    /// Keys on luma.
    #[toggle(label = "Use luma", default = true)]
    pub qual_use_lum: bool,
    /// The least luma keyed.
    #[bounded(label = "Luma low", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub qual_lum_low: f32,
    /// The most luma keyed.
    #[bounded(label = "Luma high", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub qual_lum_high: f32,
    /// How far past the range the key fades.
    #[bounded(label = "Luma softness", min = 0.0, max = 100.0, default = 10.0, unit = Percent)]
    pub qual_lum_soft: f32,
    /// The least red keyed.
    #[bounded(label = "Red low", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub qual_red_low: f32,
    /// The most red keyed.
    #[bounded(label = "Red high", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub qual_red_high: f32,
    /// The least green keyed.
    #[bounded(label = "Green low", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub qual_green_low: f32,
    /// The most green keyed.
    #[bounded(label = "Green high", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub qual_green_high: f32,
    /// The least blue keyed.
    #[bounded(label = "Blue low", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub qual_blue_low: f32,
    /// The most blue keyed.
    #[bounded(label = "Blue high", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub qual_blue_high: f32,
    /// How far past each range the key fades.
    #[bounded(label = "Softness", min = 0.0, max = 100.0, default = 10.0, unit = Percent)]
    pub qual_rgb_soft: f32,
    /// The colours a 3D key keeps: Cb and Cr, doubled, one point a sample.
    #[shape(label = "Keep samples")]
    pub qual_3d_keep: ShapePoints,
    /// Each kept sample's luma, as the first number of its point.
    #[shape(label = "Keep sample luma")]
    pub qual_3d_keep_luma: ShapePoints,
    /// The colours a 3D key leaves out, as the kept ones are stored.
    #[shape(label = "Drop samples")]
    pub qual_3d_drop: ShapePoints,
    /// Each dropped sample's luma, as the first number of its point.
    #[shape(label = "Drop sample luma")]
    pub qual_3d_drop_luma: ShapePoints,
    /// How far round each sample the 3D key reaches.
    #[bounded(label = "Size", min = 0.0, max = 100.0, default = 20.0, unit = Raw)]
    pub qual_3d_size: f32,
    /// How far past Size the 3D key fades.
    #[bounded(label = "Softness", min = 0.0, max = 100.0, default = 20.0, unit = Raw)]
    pub qual_3d_soft: f32,
    /// Softens the picture the key is judged on.
    #[bounded(label = "Denoise", min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub qual_denoise: f32,
    /// Pushes the key's greys towards black.
    #[bounded(label = "Clean black", min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub qual_clean_black: f32,
    /// Pushes the key's greys towards white.
    #[bounded(label = "Clean white", min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub qual_clean_white: f32,
    /// The least the key may be.
    #[bounded(label = "Black clip", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub qual_black_clip: f32,
    /// The most the key may be.
    #[bounded(label = "White clip", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub qual_white_clip: f32,
    /// Softens the key, px@comp.
    #[slider(
        label = "Blur radius",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Px
    )]
    pub qual_blur: f32,
    /// Grows the key outwards, or shrinks it, px@comp.
    #[slider(
        label = "Grow",
        min = -20.0,
        max = 20.0,
        default = 0.0,
        hard_min = -20.0,
        hard_max = 20.0,
        unit = Px
    )]
    pub qual_grow: f32,
    /// Keys everything the qualifier did not.
    #[toggle(label = "Invert", default = false)]
    pub qual_invert: bool,

    /// Window 1's shape. Off leaves the picture unlimited by it.
    #[choice(label = "Window 1", options = WINDOW_OPTIONS, default = 0)]
    pub win1_shape: u32,
    /// The window's centre, px@comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub win1_x: f32,
    /// px@comp; see `win1_x`.
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub win1_y: f32,
    /// The window's box, px@comp.
    #[slider(label = "Width", min = 0.0, max = 3840.0, default = 768.0, hard_min = 0.0, unit = Px)]
    pub win1_w: f32,
    /// px@comp; see `win1_w`.
    #[slider(label = "Height", min = 0.0, max = 2160.0, default = 432.0, hard_min = 0.0, unit = Px)]
    pub win1_h: f32,
    /// The box's turn, clockwise.
    #[dial(label = "Angle", default = 0.0, step = 15.0)]
    pub win1_angle: f32,
    /// A rectangle's corners, as a share of its shorter side.
    #[bounded(label = "Roundness", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub win1_round: f32,
    /// How far the edge fades outwards, px@comp.
    #[slider(label = "Outer softness", min = 0.0, max = 1000.0, default = 40.0, hard_min = 0.0, unit = Px)]
    pub win1_soft: f32,
    /// How far the edge fades inwards, px@comp.
    #[slider(label = "Inner softness", min = 0.0, max = 1000.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub win1_soft_in: f32,
    /// How much of the window keys.
    #[bounded(label = "Opacity", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub win1_opacity: f32,
    /// Keys outside the shape instead.
    #[toggle(label = "Invert", default = false)]
    pub win1_invert: bool,
    /// Cuts this window out of the ones before it.
    #[toggle(label = "Subtract", default = false)]
    pub win1_subtract: bool,
    /// A polygon's points, in -1..1 of the box.
    #[shape(label = "Points")]
    pub win1_points: ShapePoints,
    /// Draws a polygon as a smooth curve through its points.
    #[toggle(label = "Smooth", default = true)]
    pub win1_smooth: bool,

    /// Window 2's shape. Off leaves the picture unlimited by it.
    #[choice(label = "Window 2", options = WINDOW_OPTIONS, default = 0)]
    pub win2_shape: u32,
    /// The window's centre, px@comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub win2_x: f32,
    /// px@comp; see `win2_x`.
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub win2_y: f32,
    /// The window's box, px@comp.
    #[slider(label = "Width", min = 0.0, max = 3840.0, default = 768.0, hard_min = 0.0, unit = Px)]
    pub win2_w: f32,
    /// px@comp; see `win2_w`.
    #[slider(label = "Height", min = 0.0, max = 2160.0, default = 432.0, hard_min = 0.0, unit = Px)]
    pub win2_h: f32,
    /// The box's turn, clockwise.
    #[dial(label = "Angle", default = 0.0, step = 15.0)]
    pub win2_angle: f32,
    /// A rectangle's corners, as a share of its shorter side.
    #[bounded(label = "Roundness", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub win2_round: f32,
    /// How far the edge fades outwards, px@comp.
    #[slider(label = "Outer softness", min = 0.0, max = 1000.0, default = 40.0, hard_min = 0.0, unit = Px)]
    pub win2_soft: f32,
    /// How far the edge fades inwards, px@comp.
    #[slider(label = "Inner softness", min = 0.0, max = 1000.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub win2_soft_in: f32,
    /// How much of the window keys.
    #[bounded(label = "Opacity", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub win2_opacity: f32,
    /// Keys outside the shape instead.
    #[toggle(label = "Invert", default = false)]
    pub win2_invert: bool,
    /// Cuts this window out of the ones before it.
    #[toggle(label = "Subtract", default = false)]
    pub win2_subtract: bool,
    /// A polygon's points, in -1..1 of the box.
    #[shape(label = "Points")]
    pub win2_points: ShapePoints,
    /// Draws a polygon as a smooth curve through its points.
    #[toggle(label = "Smooth", default = true)]
    pub win2_smooth: bool,

    /// Window 3's shape. Off leaves the picture unlimited by it.
    #[choice(label = "Window 3", options = WINDOW_OPTIONS, default = 0)]
    pub win3_shape: u32,
    /// The window's centre, px@comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub win3_x: f32,
    /// px@comp; see `win3_x`.
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub win3_y: f32,
    /// The window's box, px@comp.
    #[slider(label = "Width", min = 0.0, max = 3840.0, default = 768.0, hard_min = 0.0, unit = Px)]
    pub win3_w: f32,
    /// px@comp; see `win3_w`.
    #[slider(label = "Height", min = 0.0, max = 2160.0, default = 432.0, hard_min = 0.0, unit = Px)]
    pub win3_h: f32,
    /// The box's turn, clockwise.
    #[dial(label = "Angle", default = 0.0, step = 15.0)]
    pub win3_angle: f32,
    /// A rectangle's corners, as a share of its shorter side.
    #[bounded(label = "Roundness", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub win3_round: f32,
    /// How far the edge fades outwards, px@comp.
    #[slider(label = "Outer softness", min = 0.0, max = 1000.0, default = 40.0, hard_min = 0.0, unit = Px)]
    pub win3_soft: f32,
    /// How far the edge fades inwards, px@comp.
    #[slider(label = "Inner softness", min = 0.0, max = 1000.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub win3_soft_in: f32,
    /// How much of the window keys.
    #[bounded(label = "Opacity", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub win3_opacity: f32,
    /// Keys outside the shape instead.
    #[toggle(label = "Invert", default = false)]
    pub win3_invert: bool,
    /// Cuts this window out of the ones before it.
    #[toggle(label = "Subtract", default = false)]
    pub win3_subtract: bool,
    /// A polygon's points, in -1..1 of the box.
    #[shape(label = "Points")]
    pub win3_points: ShapePoints,
    /// Draws a polygon as a smooth curve through its points.
    #[toggle(label = "Smooth", default = true)]
    pub win3_smooth: bool,

    /// Window 4's shape. Off leaves the picture unlimited by it.
    #[choice(label = "Window 4", options = WINDOW_OPTIONS, default = 0)]
    pub win4_shape: u32,
    /// The window's centre, px@comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub win4_x: f32,
    /// px@comp; see `win4_x`.
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub win4_y: f32,
    /// The window's box, px@comp.
    #[slider(label = "Width", min = 0.0, max = 3840.0, default = 768.0, hard_min = 0.0, unit = Px)]
    pub win4_w: f32,
    /// px@comp; see `win4_w`.
    #[slider(label = "Height", min = 0.0, max = 2160.0, default = 432.0, hard_min = 0.0, unit = Px)]
    pub win4_h: f32,
    /// The box's turn, clockwise.
    #[dial(label = "Angle", default = 0.0, step = 15.0)]
    pub win4_angle: f32,
    /// A rectangle's corners, as a share of its shorter side.
    #[bounded(label = "Roundness", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub win4_round: f32,
    /// How far the edge fades outwards, px@comp.
    #[slider(label = "Outer softness", min = 0.0, max = 1000.0, default = 40.0, hard_min = 0.0, unit = Px)]
    pub win4_soft: f32,
    /// How far the edge fades inwards, px@comp.
    #[slider(label = "Inner softness", min = 0.0, max = 1000.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub win4_soft_in: f32,
    /// How much of the window keys.
    #[bounded(label = "Opacity", min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub win4_opacity: f32,
    /// Keys outside the shape instead.
    #[toggle(label = "Invert", default = false)]
    pub win4_invert: bool,
    /// Cuts this window out of the ones before it.
    #[toggle(label = "Subtract", default = false)]
    pub win4_subtract: bool,
    /// A polygon's points, in -1..1 of the box.
    #[shape(label = "Points")]
    pub win4_points: ShapePoints,
    /// Draws a polygon as a smooth curve through its points.
    #[toggle(label = "Smooth", default = true)]
    pub win4_smooth: bool,

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

/// Raster pixels per comp pixel, for the blurs whose size is fixed at comp
/// size.
pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

/// Correction's behaviour.
pub struct CorrectionDef;

impl EffectDef for CorrectionDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Correction as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let pk = Correction::read(p).packed(p.float(DERIVED_PX_SCALE, 1.0));
        cpu::correction(rgba, w, h, &pk, &[]);
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::model::{EffectInstance, EffectValue};

    /// A window's outline comes back from the saved file as it went in.
    #[test]
    fn a_shape_survives_save_and_open() {
        let mut e = crate::fx::builtins::instantiate("correction").unwrap();
        let drawn = vec![[-0.5, -0.5], [0.5, -0.25], [0.0, 0.75]];
        let row = e.params.iter_mut().find(|p| p.id == "win1_points").unwrap();
        assert_eq!(row.value, EffectValue::Shape(Vec::new()));
        row.value = EffectValue::Shape(drawn.clone());
        let text = serde_json::to_string(&e).unwrap();
        let back: EffectInstance = serde_json::from_str(&text).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.param("win1_points"), Some(&EffectValue::Shape(drawn)));
    }
}
