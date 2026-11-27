mod geometry;
mod noise;

use crate::fx::{
    params::{CurvePoints, ParamId, Value},
    registry::{EffectDef, EffectMetadata, ResolveCx},
    schema::{EffectSchema, ParamGroup},
};
use lumit_fx_macros::Effect;

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    collapsed: bool,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

const GROUPS: &[ParamGroup] = &[
    group("Stroke spacing", &["distribution_ramp"], false),
    group(
        "Path warp",
        &[
            "distortion_coordinates",
            "distortion_amplitude",
            "distortion_frequency",
            "distortion_scale_x",
            "distortion_scale_y",
            "distortion_offset_x",
            "distortion_offset_y",
            "distortion_evolution",
            "distortion_loop",
            "distortion_line_offset",
            "distortion_seed",
            "distortion_ramp",
            "distortion_ramp_offset",
            "distortion_ramp_scale",
            "distortion_ramp_loop",
        ],
        true,
    ),
    group(
        "Mark shape",
        &[
            "primitives",
            "point_segments",
            "visible_points",
            "angle",
            "rotation_along_lines",
        ],
        false,
    ),
    group(
        "Heading variation",
        &[
            "rotation_noise_coordinates",
            "rotation_noise_amplitude",
            "rotation_noise_frequency",
            "rotation_noise_scale_x",
            "rotation_noise_scale_y",
            "rotation_noise_offset_x",
            "rotation_noise_offset_y",
            "rotation_noise_evolution",
            "rotation_noise_loop",
            "rotation_noise_line_offset",
            "rotation_noise_seed",
            "rotation_noise_ramp",
            "rotation_noise_ramp_offset",
            "rotation_noise_ramp_scale",
            "rotation_noise_ramp_loop",
        ],
        true,
    ),
    group(
        "Coverage texture",
        &[
            "alpha_noise_coordinates",
            "alpha_noise_amplitude",
            "alpha_noise_frequency",
            "alpha_noise_scale_x",
            "alpha_noise_scale_y",
            "alpha_noise_offset_x",
            "alpha_noise_offset_y",
            "alpha_noise_evolution",
            "alpha_noise_loop",
            "alpha_noise_line_offset",
            "alpha_noise_seed",
            "alpha_noise_brightness",
            "alpha_noise_ramp",
            "alpha_noise_ramp_offset",
            "alpha_noise_ramp_scale",
            "alpha_noise_ramp_loop",
        ],
        true,
    ),
    group(
        "Coverage wave",
        &[
            "alpha_sinus_amplitude",
            "alpha_sinus_frequency",
            "alpha_sinus_offset",
            "alpha_sinus_line_offset",
            "alpha_sinus_constant",
        ],
        true,
    ),
    group(
        "Path trim",
        &["stroke_offset", "stroke_begin", "stroke_end", "stroke_loop"],
        true,
    ),
    group(
        "Stroke tips",
        &[
            "cap_begin",
            "arrow_length_begin",
            "arrow_angle_begin",
            "cap_end",
            "arrow_length_end",
            "arrow_angle_end",
        ],
        true,
    ),
    group(
        "Stroke variation",
        &[
            "distribution_random",
            "offset_random",
            "gradient_random",
            "thickness_random",
            "opacity_random",
            "random_seed",
        ],
        true,
    ),
    group(
        "Across-stroke colour",
        &[
            "gradient_blending",
            "cross_colours_count",
            "cross_colour_1",
            "cross_colour_2",
            "cross_colour_3",
            "cross_colour_4",
            "cross_colour_5",
            "cross_gradient",
            "cross_gradient_offset",
            "cross_gradient_scale",
            "cross_gradient_loop",
        ],
        true,
    ),
    group(
        "Brush texture",
        &[
            "dry_brush",
            "dry_brush_ramp",
            "dry_brush_scale",
            "edge_roughness",
            "edge_roughness_depth",
            "edge_roughness_scale",
            "coverage_hardness",
        ],
        true,
    ),
    group(
        "Paint density colour",
        &[
            "density_colour",
            "density_colour_thin",
            "density_colour_mid",
            "density_colour_dense",
        ],
        true,
    ),
    group(
        "First guide",
        &[
            "mask_1",
            "mask_1_feather",
            "mask_1_density",
            "mask_1_thickness",
            "mask_1_thickness_ramp",
            "mask_1_thickness_ramp_offset",
            "mask_1_thickness_ramp_scale",
            "mask_1_thickness_ramp_loop",
            "mask_1_invert",
            "mask_1_opacity",
            "mask_1_opacity_ramp",
            "mask_1_opacity_ramp_offset",
            "mask_1_opacity_ramp_scale",
            "mask_1_opacity_ramp_loop",
            "mask_1_colours_count",
            "mask_1_colour_1",
            "mask_1_colour_2",
            "mask_1_colour_3",
            "mask_1_colour_4",
            "mask_1_colour_5",
            "mask_1_gradient",
            "mask_1_gradient_offset",
            "mask_1_gradient_random",
            "mask_1_gradient_loop",
            "mask_1_sine_operation",
            "mask_1_sine_1_amplitude",
            "mask_1_sine_1_frequency",
            "mask_1_sine_1_offset",
            "mask_1_sine_2_amplitude",
            "mask_1_sine_2_frequency",
            "mask_1_sine_2_offset",
        ],
        false,
    ),
    group(
        "Second guide",
        &[
            "mask_2",
            "clone_mask_1",
            "mask_2_feather",
            "mask_2_density",
            "mask_2_thickness",
            "mask_2_thickness_ramp",
            "mask_2_thickness_ramp_offset",
            "mask_2_thickness_ramp_scale",
            "mask_2_thickness_ramp_loop",
            "mask_2_invert",
            "mask_2_opacity",
            "mask_2_opacity_ramp",
            "mask_2_opacity_ramp_offset",
            "mask_2_opacity_ramp_scale",
            "mask_2_opacity_ramp_loop",
            "mask_2_colours_count",
            "mask_2_colour_1",
            "mask_2_colour_2",
            "mask_2_colour_3",
            "mask_2_colour_4",
            "mask_2_colour_5",
            "mask_2_gradient",
            "mask_2_gradient_offset",
            "mask_2_gradient_random",
            "mask_2_gradient_loop",
            "mask_2_sine_operation",
            "mask_2_sine_1_amplitude",
            "mask_2_sine_1_frequency",
            "mask_2_sine_1_offset",
            "mask_2_sine_2_amplitude",
            "mask_2_sine_2_frequency",
            "mask_2_sine_2_offset",
        ],
        false,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "turbiniu",
    label = "Turbiniu",
    version = 4,
    category = Generate,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    groups = GROUPS,
)]
pub struct Turbiniu {
    #[choice(label = "Overlap mode", options = ["Source over", "Additive"], default = 0)]
    pub blending: u32,

    #[choice(label = "Edge sampling", options = ["Single sample", "Five samples"], default = 1)]
    pub multisampling: u32,

    #[counter(label = "Stroke count", min = 1, max = 100, default = 3, hard_min = 1, hard_max = 10000, unit = Raw)]
    pub iterations: i32,

    #[counter(label = "Path resolution", min = 1, max = 250, default = 250, hard_min = 0, hard_max = 10000, unit = Raw)]
    pub curve_segments: i32,

    #[curve(label = "Spacing profile")]
    pub distribution_ramp: CurvePoints,

    #[choice(label = "Sampling domain", options = ["Image coordinates", "Path coordinates", "Sample order"], default = 1)]
    pub distortion_coordinates: u32,

    #[slider(label = "Warp distance", min = 0.0, max = 10.0, default = 0.0, unit = Px, hard_min = 0.0)]
    pub distortion_amplitude: f32,

    #[slider(label = "Pattern frequency", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub distortion_frequency: f32,

    #[slider(label = "Horizontal stretch", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub distortion_scale_x: f32,

    #[slider(label = "Vertical stretch", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub distortion_scale_y: f32,

    #[slider(label = "Horizontal shift", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub distortion_offset_x: f32,

    #[slider(label = "Vertical shift", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub distortion_offset_y: f32,

    #[slider(label = "Pattern phase", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub distortion_evolution: f32,

    #[slider(label = "Phase period", min = 0.0, max = 10.0, default = 0.0, unit = Raw, hard_min = 0.0)]
    pub distortion_loop: f32,

    #[slider(label = "Horizontal step per stroke", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub distortion_line_offset: f32,

    #[counter(label = "Pattern seed", min = 0, max = 1000, default = 0, hard_min = 0, hard_max = 32768, unit = Raw)]
    pub distortion_seed: i32,

    #[curve(label = "Strength profile", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub distortion_ramp: CurvePoints,

    #[slider(label = "Profile shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub distortion_ramp_offset: f32,

    #[slider(label = "Profile span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub distortion_ramp_scale: f32,

    #[toggle(label = "Repeat profile", default = false)]
    pub distortion_ramp_loop: bool,

    #[choice(label = "Draw as", options = ["Strokes", "Markers"], default = 0)]
    pub primitives: u32,

    #[counter(label = "Marker sides", min = 3, max = 32, default = 8, hard_min = 3, hard_max = 128, unit = Raw)]
    pub point_segments: i32,

    #[counter(label = "Marker fill rate", min = 0, max = 100, default = 100, hard_min = 0, hard_max = 100, unit = Raw)]
    pub visible_points: i32,

    #[dial(label = "Marker heading", default = 0.0)]
    pub angle: f32,

    #[slider(label = "Follow tangent", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub rotation_along_lines: f32,

    #[choice(label = "Sampling domain", options = ["Image coordinates", "Path coordinates", "Sample order"], default = 1)]
    pub rotation_noise_coordinates: u32,

    #[slider(label = "Heading deviation", min = 0.0, max = 10.0, default = 0.0, unit = Degrees, hard_min = 0.0)]
    pub rotation_noise_amplitude: f32,

    #[slider(label = "Pattern frequency", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub rotation_noise_frequency: f32,

    #[slider(label = "Horizontal stretch", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub rotation_noise_scale_x: f32,

    #[slider(label = "Vertical stretch", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub rotation_noise_scale_y: f32,

    #[slider(label = "Horizontal shift", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub rotation_noise_offset_x: f32,

    #[slider(label = "Vertical shift", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub rotation_noise_offset_y: f32,

    #[slider(label = "Pattern phase", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub rotation_noise_evolution: f32,

    #[slider(label = "Phase period", min = 0.0, max = 10.0, default = 0.0, unit = Raw, hard_min = 0.0)]
    pub rotation_noise_loop: f32,

    #[slider(label = "Phase step per stroke", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub rotation_noise_line_offset: f32,

    #[counter(label = "Pattern seed", min = 0, max = 1000, default = 0, hard_min = 0, hard_max = 32768, unit = Raw)]
    pub rotation_noise_seed: i32,

    #[curve(label = "Strength profile", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub rotation_noise_ramp: CurvePoints,

    #[slider(label = "Profile shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub rotation_noise_ramp_offset: f32,

    #[slider(label = "Profile span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub rotation_noise_ramp_scale: f32,

    #[toggle(label = "Repeat profile", default = false)]
    pub rotation_noise_ramp_loop: bool,

    #[choice(label = "Sampling domain", options = ["Image coordinates", "Path coordinates", "Sample order"], default = 1)]
    pub alpha_noise_coordinates: u32,

    #[slider(label = "Coverage variation", min = 0.0, max = 10.0, default = 0.0, unit = Raw, hard_min = 0.0)]
    pub alpha_noise_amplitude: f32,

    #[slider(label = "Pattern frequency", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub alpha_noise_frequency: f32,

    #[slider(label = "Horizontal stretch", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub alpha_noise_scale_x: f32,

    #[slider(label = "Vertical stretch", min = 0.0, max = 10.0, default = 1.0, unit = Raw, hard_min = 0.0)]
    pub alpha_noise_scale_y: f32,

    #[slider(label = "Horizontal shift", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub alpha_noise_offset_x: f32,

    #[slider(label = "Vertical shift", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub alpha_noise_offset_y: f32,

    #[slider(label = "Pattern phase", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub alpha_noise_evolution: f32,

    #[slider(label = "Phase period", min = 0.0, max = 10.0, default = 0.0, unit = Raw, hard_min = 0.0)]
    pub alpha_noise_loop: f32,

    #[slider(label = "Phase step per stroke", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub alpha_noise_line_offset: f32,

    #[counter(label = "Pattern seed", min = 0, max = 1000, default = 0, hard_min = 0, hard_max = 32768, unit = Raw)]
    pub alpha_noise_seed: i32,

    #[slider(label = "Base coverage", min = -1.0, max = 1.0, default = 1.0, unit = Raw, hard_min = -2.0, hard_max = 2.0)]
    pub alpha_noise_brightness: f32,

    #[curve(label = "Strength profile", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub alpha_noise_ramp: CurvePoints,

    #[slider(label = "Profile shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub alpha_noise_ramp_offset: f32,

    #[slider(label = "Profile span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub alpha_noise_ramp_scale: f32,

    #[toggle(label = "Repeat profile", default = false)]
    pub alpha_noise_ramp_loop: bool,

    #[slider(label = "Wave depth", min = -1.0, max = 1.0, default = 0.0, unit = Raw)]
    pub alpha_sinus_amplitude: f32,

    #[slider(label = "Wave rate", min = -20.0, max = 20.0, default = 0.0, unit = Raw, hard_min = -1000.0, hard_max = 1000.0)]
    pub alpha_sinus_frequency: f32,

    #[slider(label = "Wave phase", min = -1.0, max = 1.0, default = 0.0, unit = Raw)]
    pub alpha_sinus_offset: f32,

    #[slider(label = "Phase step per stroke", min = -1.0, max = 1.0, default = 0.0, unit = Raw)]
    pub alpha_sinus_line_offset: f32,

    #[slider(label = "Coverage bias", min = -1.0, max = 1.0, default = 0.0, unit = Raw)]
    pub alpha_sinus_constant: f32,

    #[slider(label = "Path travel", min = -100.0, max = 100.0, default = 0.0, unit = Percent, hard_min = -10000.0, hard_max = 10000.0)]
    pub stroke_offset: f32,

    #[slider(label = "Trim in", min = -100.0, max = 100.0, default = 0.0, unit = Percent, hard_min = -100.0, hard_max = 100.0)]
    pub stroke_begin: f32,

    #[slider(label = "Trim out", min = 0.0, max = 100.0, default = 100.0, unit = Percent, hard_min = 0.0, hard_max = 200.0)]
    pub stroke_end: f32,

    #[toggle(label = "Wrap trim", default = false)]
    pub stroke_loop: bool,

    #[choice(label = "Leading tip", options = ["Flat", "Solid arrow", "Open arrow", "Rounded"], default = 0)]
    pub cap_begin: u32,

    #[slider(label = "Leading tip reach", min = 0.0, max = 100.0, default = 15.0, unit = Px, hard_min = 0.0, hard_max = 1000.0)]
    pub arrow_length_begin: f32,

    #[slider(label = "Leading tip spread", min = -180.0, max = 180.0, default = 45.0, unit = Degrees, hard_min = -180.0, hard_max = 180.0)]
    pub arrow_angle_begin: f32,

    #[choice(label = "Trailing tip", options = ["Flat", "Solid arrow", "Open arrow", "Rounded"], default = 0)]
    pub cap_end: u32,

    #[slider(label = "Trailing tip reach", min = 0.0, max = 100.0, default = 15.0, unit = Px, hard_min = 0.0, hard_max = 1000.0)]
    pub arrow_length_end: f32,

    #[slider(label = "Trailing tip spread", min = -180.0, max = 180.0, default = 45.0, unit = Degrees, hard_min = -180.0, hard_max = 180.0)]
    pub arrow_angle_end: f32,

    #[slider(label = "Spacing jitter", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub distribution_random: f32,

    #[slider(label = "Travel jitter", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub offset_random: f32,

    #[slider(label = "Palette jitter", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub gradient_random: f32,

    #[slider(label = "Width jitter", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub thickness_random: f32,

    #[slider(label = "Coverage jitter", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub opacity_random: f32,

    #[counter(label = "Variation seed", min = 0, max = 1000, default = 0, hard_min = 0, hard_max = 32768, unit = Raw)]
    pub random_seed: i32,

    #[choice(label = "Palette combine", options = ["Sum", "Product"], default = 0)]
    pub gradient_blending: u32,

    #[counter(label = "Palette size", min = 1, max = 5, default = 1, hard_min = 1, hard_max = 5, unit = Raw)]
    pub cross_colours_count: i32,

    #[colour(label = "Palette swatch 1", default = [0.0, 0.0, 0.0, 0.0], max = 4.0)]
    pub cross_colour_1: [f32; 4],

    #[colour(label = "Palette swatch 2", default = [0.0, 0.0, 0.0, 0.0], max = 4.0)]
    pub cross_colour_2: [f32; 4],

    #[colour(label = "Palette swatch 3", default = [0.0, 0.0, 0.0, 0.0], max = 4.0)]
    pub cross_colour_3: [f32; 4],

    #[colour(label = "Palette swatch 4", default = [0.0, 0.0, 0.0, 0.0], max = 4.0)]
    pub cross_colour_4: [f32; 4],

    #[colour(label = "Palette swatch 5", default = [0.0, 0.0, 0.0, 0.0], max = 4.0)]
    pub cross_colour_5: [f32; 4],

    #[curve(label = "Across-stroke colour profile")]
    pub cross_gradient: CurvePoints,

    #[slider(label = "Across-stroke palette shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub cross_gradient_offset: f32,

    #[slider(label = "Across-stroke palette span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub cross_gradient_scale: f32,

    #[toggle(label = "Repeat across-stroke palette", default = false)]
    pub cross_gradient_loop: bool,

    #[slider(label = "Dry brush", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub dry_brush: f32,

    #[curve(label = "Dry brush profile", default = [[0.0, 0.0], [1.0, 1.0]])]
    pub dry_brush_ramp: CurvePoints,

    #[slider(label = "Dry brush grain", min = 1.0, max = 200.0, default = 40.0, unit = Px, hard_min = 0.5, hard_max = 10000.0)]
    pub dry_brush_scale: f32,

    #[slider(label = "Edge roughness", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub edge_roughness: f32,

    #[slider(label = "Rough edge depth", min = 0.0, max = 100.0, default = 35.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub edge_roughness_depth: f32,

    #[slider(label = "Rough edge grain", min = 1.0, max = 100.0, default = 12.0, unit = Px, hard_min = 0.5, hard_max = 10000.0)]
    pub edge_roughness_scale: f32,

    #[slider(label = "Coverage hardness", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub coverage_hardness: f32,

    #[slider(label = "Density colour", min = 0.0, max = 100.0, default = 0.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub density_colour: f32,

    #[colour(label = "Thin paint colour", default = [0.8, 0.45, 0.15, 1.0], max = 4.0)]
    pub density_colour_thin: [f32; 4],

    #[colour(label = "Mid paint colour", default = [0.55, 0.08, 0.05, 1.0], max = 4.0)]
    pub density_colour_mid: [f32; 4],

    #[colour(label = "Dense paint colour", default = [0.05, 0.02, 0.03, 1.0], max = 4.0)]
    pub density_colour_dense: [f32; 4],

    #[mask_path(label = "First path")]
    pub mask_1: bool,

    #[slider(label = "Edge softness", min = 0.0, max = 10.0, default = 1.0, unit = Px, hard_min = 0.0, hard_max = 1000.0)]
    pub mask_1_feather: f32,

    #[curve(label = "Sample spacing profile")]
    pub mask_1_density: CurvePoints,

    #[slider(label = "Stroke width", min = 0.0, max = 10.0, default = 1.0, unit = Px, hard_min = 0.0)]
    pub mask_1_thickness: f32,

    #[curve(label = "Width profile", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub mask_1_thickness_ramp: CurvePoints,

    #[slider(label = "Width profile shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_1_thickness_ramp_offset: f32,

    #[slider(label = "Width profile span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub mask_1_thickness_ramp_scale: f32,

    #[toggle(label = "Repeat width profile", default = false)]
    pub mask_1_thickness_ramp_loop: bool,

    #[toggle(label = "Reverse guide", default = false)]
    pub mask_1_invert: bool,

    #[slider(label = "Base coverage", min = 0.0, max = 1.0, default = 1.0, unit = Raw, hard_min = 0.0, hard_max = 1.0)]
    pub mask_1_opacity: f32,

    #[curve(label = "Coverage profile", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub mask_1_opacity_ramp: CurvePoints,

    #[slider(label = "Coverage profile shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_1_opacity_ramp_offset: f32,

    #[slider(label = "Coverage profile span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub mask_1_opacity_ramp_scale: f32,

    #[toggle(label = "Repeat coverage profile", default = false)]
    pub mask_1_opacity_ramp_loop: bool,

    #[counter(label = "Palette size", min = 1, max = 5, default = 1, hard_min = 1, hard_max = 5, unit = Raw)]
    pub mask_1_colours_count: i32,

    #[colour(label = "Palette swatch 1", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_1_colour_1: [f32; 4],

    #[colour(label = "Palette swatch 2", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_1_colour_2: [f32; 4],

    #[colour(label = "Palette swatch 3", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_1_colour_3: [f32; 4],

    #[colour(label = "Palette swatch 4", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_1_colour_4: [f32; 4],

    #[colour(label = "Palette swatch 5", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_1_colour_5: [f32; 4],

    #[curve(label = "Along-path colour profile")]
    pub mask_1_gradient: CurvePoints,

    #[slider(label = "Along-path palette shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_1_gradient_offset: f32,

    #[slider(label = "Palette shift jitter", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_1_gradient_random: f32,

    #[toggle(label = "Repeat along-path palette", default = false)]
    pub mask_1_gradient_loop: bool,

    #[choice(label = "Wave combine", options = ["Sum", "Product"], default = 0)]
    pub mask_1_sine_operation: u32,

    #[slider(label = "Wave 1 depth", min = -100.0, max = 100.0, default = 0.0, unit = Px, hard_min = -10000.0, hard_max = 10000.0)]
    pub mask_1_sine_1_amplitude: f32,

    #[slider(label = "Wave 1 rate", min = -10.0, max = 10.0, default = 1.0, unit = Raw)]
    pub mask_1_sine_1_frequency: f32,

    #[slider(label = "Wave 1 phase", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub mask_1_sine_1_offset: f32,

    #[slider(label = "Wave 2 depth", min = -100.0, max = 100.0, default = 0.0, unit = Px, hard_min = -10000.0, hard_max = 10000.0)]
    pub mask_1_sine_2_amplitude: f32,

    #[slider(label = "Wave 2 rate", min = -10.0, max = 10.0, default = 1.0, unit = Raw)]
    pub mask_1_sine_2_frequency: f32,

    #[slider(label = "Wave 2 phase", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub mask_1_sine_2_offset: f32,

    #[mask_path(label = "Second path", self_default = false)]
    pub mask_2: bool,

    #[toggle(label = "Inherit first guide style", default = false)]
    pub clone_mask_1: bool,

    #[slider(label = "Edge softness", min = 0.0, max = 10.0, default = 1.0, unit = Px, hard_min = 0.0, hard_max = 1000.0)]
    pub mask_2_feather: f32,

    #[curve(label = "Sample spacing profile")]
    pub mask_2_density: CurvePoints,

    #[slider(label = "Stroke width", min = 0.0, max = 10.0, default = 1.0, unit = Px, hard_min = 0.0)]
    pub mask_2_thickness: f32,

    #[curve(label = "Width profile", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub mask_2_thickness_ramp: CurvePoints,

    #[slider(label = "Width profile shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_2_thickness_ramp_offset: f32,

    #[slider(label = "Width profile span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub mask_2_thickness_ramp_scale: f32,

    #[toggle(label = "Repeat width profile", default = false)]
    pub mask_2_thickness_ramp_loop: bool,

    #[toggle(label = "Reverse guide", default = false)]
    pub mask_2_invert: bool,

    #[slider(label = "Base coverage", min = 0.0, max = 1.0, default = 1.0, unit = Raw, hard_min = 0.0, hard_max = 1.0)]
    pub mask_2_opacity: f32,

    #[curve(label = "Coverage profile", default = [[0.0, 1.0], [1.0, 1.0]])]
    pub mask_2_opacity_ramp: CurvePoints,

    #[slider(label = "Coverage profile shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_2_opacity_ramp_offset: f32,

    #[slider(label = "Coverage profile span", min = -100.0, max = 100.0, default = 100.0, unit = Percent)]
    pub mask_2_opacity_ramp_scale: f32,

    #[toggle(label = "Repeat coverage profile", default = false)]
    pub mask_2_opacity_ramp_loop: bool,

    #[counter(label = "Palette size", min = 1, max = 5, default = 1, hard_min = 1, hard_max = 5, unit = Raw)]
    pub mask_2_colours_count: i32,

    #[colour(label = "Palette swatch 1", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_2_colour_1: [f32; 4],

    #[colour(label = "Palette swatch 2", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_2_colour_2: [f32; 4],

    #[colour(label = "Palette swatch 3", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_2_colour_3: [f32; 4],

    #[colour(label = "Palette swatch 4", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_2_colour_4: [f32; 4],

    #[colour(label = "Palette swatch 5", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub mask_2_colour_5: [f32; 4],

    #[curve(label = "Along-path colour profile")]
    pub mask_2_gradient: CurvePoints,

    #[slider(label = "Along-path palette shift", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_2_gradient_offset: f32,

    #[slider(label = "Palette shift jitter", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub mask_2_gradient_random: f32,

    #[toggle(label = "Repeat along-path palette", default = false)]
    pub mask_2_gradient_loop: bool,

    #[choice(label = "Wave combine", options = ["Sum", "Product"], default = 0)]
    pub mask_2_sine_operation: u32,

    #[slider(label = "Wave 1 depth", min = -100.0, max = 100.0, default = 0.0, unit = Px, hard_min = -10000.0, hard_max = 10000.0)]
    pub mask_2_sine_1_amplitude: f32,

    #[slider(label = "Wave 1 rate", min = -10.0, max = 10.0, default = 1.0, unit = Raw)]
    pub mask_2_sine_1_frequency: f32,

    #[slider(label = "Wave 1 phase", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub mask_2_sine_1_offset: f32,

    #[slider(label = "Wave 2 depth", min = -100.0, max = 100.0, default = 0.0, unit = Px, hard_min = -10000.0, hard_max = 10000.0)]
    pub mask_2_sine_2_amplitude: f32,

    #[slider(label = "Wave 2 rate", min = -10.0, max = 10.0, default = 1.0, unit = Raw)]
    pub mask_2_sine_2_frequency: f32,

    #[slider(label = "Wave 2 phase", min = -10.0, max = 10.0, default = 0.0, unit = Raw)]
    pub mask_2_sine_2_offset: f32,

    #[slider(label = "Effect amount", min = 0.0, max = 100.0, default = 100.0, unit = Percent, hard_min = 0.0, hard_max = 100.0)]
    pub mix: f32,
}

impl Turbiniu {
    pub const PX_SCALE: ParamId = ParamId::new("derived.px_scale");
}

pub struct TurbiniuDef;

impl EffectDef for TurbiniuDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Turbiniu as EffectMetadata>::SCHEMA
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Turbiniu::PX_SCALE, Value::Float(cx.px_scale));
    }
}
