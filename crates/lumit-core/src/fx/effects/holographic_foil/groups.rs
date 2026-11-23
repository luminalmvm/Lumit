//! Holographic foil's groups and the rules that grey its rows.

use super::lights;
use crate::fx::schema::{EnabledCond, EnabledWhen, ParamGroup};

/// The Relief choice that builds a bevel from the sheet's alpha.
const BEVEL: u32 = super::surface::BEVEL_SOURCE;

/// The Pattern choice whose grooves the Direction map steers.
const FROM_MAP: u32 = 3;

/// Shown only while Lighting is Custom.
const CUSTOM_RIG: Option<(&str, &[u32])> = Some(("lighting", &[lights::CUSTOM]));

/// Shown only while Material is Vortex.
const VORTEX_ONLY: Option<(&str, &[u32])> = Some(("material", &[super::VORTEX]));

/// Shown only while Material is Shatter.
const SHATTER_ONLY: Option<(&str, &[u32])> = Some(("material", &[super::SHATTER]));

/// Shown only while Material is Glitter.
const GLITTER_ONLY: Option<(&str, &[u32])> = Some(("material", &[super::GLITTER]));

/// Shown only while Material is Sparkle.
const SPARKLE_ONLY: Option<(&str, &[u32])> = Some(("material", &[super::SPARKLE]));

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

const fn custom(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: true,
        visible_when: CUSTOM_RIG,
        visible_when_lens_elements: None,
    }
}

/// The lights' rows only show under Custom: every other Lighting choice puts
/// its own rig in their place when the effect is drawn.
pub const HOLOGRAPHIC_FOIL_GROUPS: &[ParamGroup] = &[
    group(
        "Tilt",
        &[
            "tilt_x",
            "tilt_y",
            "spin",
            "auto_tilt",
            "auto_tilt_speed",
            "follow_rotation",
            "motion_blur",
        ],
        false,
    ),
    group("Sheet", &["sheet_width", "view_distance"], true),
    group(
        "Grating",
        &[
            "pitch",
            "groove_angle",
            "pattern",
            "centre_x",
            "centre_y",
            "depth",
            "coverage",
            "fan",
            "softness",
            "orientation_jitter",
            "pitch_jitter",
            "lacquer",
        ],
        false,
    ),
    ParamGroup {
        label: "Vortex",
        params: &["swirl", "handedness", "bands", "band_contrast"],
        collapsed: false,
        visible_when: VORTEX_ONLY,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Shatter",
        params: &[
            "shard_size",
            "irregularity",
            "splits",
            "directions",
            "shard_tilt",
            "pitch_a",
            "pitch_b",
            "pitch_c",
            "chirp",
            "ridge_height",
            "ridge_width",
            "creased",
        ],
        collapsed: false,
        visible_when: SHATTER_ONLY,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Glitter",
        params: &[
            "flake_size",
            "flake_shape",
            "flake_layers",
            "flake_tilt",
            "flake_curve",
            "pitch_min",
            "pitch_max",
            "star_size",
            "star_density",
            "base_colour",
        ],
        collapsed: false,
        visible_when: GLITTER_ONLY,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Sparkle",
        params: &[
            "dot_size",
            "dot_cover",
            "dot_irregularity",
            "dot_tilt",
            "dot_dome",
            "speck_share",
            "speck_peak",
            "speck_width",
            "speck_floor",
            "base_matte",
            "sand_size",
            "sand_tilt",
        ],
        collapsed: false,
        visible_when: SPARKLE_ONLY,
        visible_when_lens_elements: None,
    },
    group(
        "Tint",
        &["tint", "tint_pass", "tint_edge", "tint_width", "tint_floor"],
        true,
    ),
    group("Lights", &["lighting", "rig_rotation"], false),
    custom(
        "Spot A",
        &[
            "spot_a_on",
            "spot_a_x",
            "spot_a_y",
            "spot_a_size",
            "spot_a_intensity",
            "spot_a_distance",
            "spot_a_spectrum",
        ],
    ),
    custom(
        "Spot B",
        &[
            "spot_b_on",
            "spot_b_x",
            "spot_b_y",
            "spot_b_size",
            "spot_b_intensity",
            "spot_b_distance",
            "spot_b_spectrum",
        ],
    ),
    custom(
        "Spot C",
        &[
            "spot_c_on",
            "spot_c_x",
            "spot_c_y",
            "spot_c_size",
            "spot_c_intensity",
            "spot_c_distance",
            "spot_c_spectrum",
        ],
    ),
    custom(
        "Panel",
        &[
            "panel_on",
            "panel_x",
            "panel_y",
            "panel_width",
            "panel_length",
            "panel_intensity",
            "panel_distance",
            "panel_spectrum",
        ],
    ),
    custom("Surroundings", &["rims", "fill", "scrim", "room"]),
    group(
        "Surface",
        &[
            "coat_roughness",
            "relief_source",
            "relief",
            "relief_scale",
            "relief_on",
            "relief_invert",
            "bevel_width",
            "bevel_height",
            "bevel_profile",
            "bevel_softness",
            "waviness",
            "orange_peel",
            "lines",
            "line_pitch",
            "line_height",
            "bow",
            "bow_axis",
            "wear",
        ],
        true,
    ),
    group(
        "Maps",
        &[
            "height_map",
            "height_map_placement",
            "height_map_scale",
            "height_map_wrap",
            "foil_mask",
            "foil_mask_placement",
            "direction_map",
            "direction_map_placement",
            "direction_mode",
            "pitch_map",
            "pitch_map_placement",
            "pitch_white",
            "environment",
            "environment_amount",
        ],
        true,
    ),
    group(
        "Output",
        &["exposure", "highlights", "quality", "seed"],
        true,
    ),
];

const fn when(param: &'static str, on: &'static str, cond: EnabledCond) -> EnabledWhen {
    EnabledWhen { param, on, cond }
}

/// A linear grating has a groove angle, and only a custom dye has an edge.
/// The bevel's rows serve only the bevel, which has no strength, scale or
/// invert of its own. A vortex sets its own grooves and their strays; shards
/// set their own grooves and pitches and have no centre. Flakes set their own
/// grooves, pitches and spread. A Height map layer stands in for the built-in
/// relief and the bevel, and each map's own rows wait for its layer. The
/// Direction map steers only grooves Pattern lays From map, which a vortex,
/// shards and flakes never do, and the Pitch map is lost on shards and
/// flakes. The centre stays live under Linear, since a vortex reads it
/// whatever the pattern.
pub const HOLOGRAPHIC_FOIL_ENABLED_WHEN: &[EnabledWhen] = &[
    when("groove_angle", "pattern", EnabledCond::ChoiceIs(0)),
    when(
        "groove_angle",
        "material",
        EnabledCond::ChoiceIsNot(super::VORTEX),
    ),
    when(
        "pattern",
        "material",
        EnabledCond::ChoiceIsNot(super::VORTEX),
    ),
    when(
        "orientation_jitter",
        "material",
        EnabledCond::ChoiceIsNot(super::VORTEX),
    ),
    when(
        "pitch_jitter",
        "material",
        EnabledCond::ChoiceIsNot(super::VORTEX),
    ),
    when(
        "pitch",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "groove_angle",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "pattern",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "centre_x",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "centre_y",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "orientation_jitter",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "pitch_jitter",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "pitch",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "groove_angle",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "pattern",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "centre_x",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "centre_y",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when("fan", "material", EnabledCond::ChoiceIsNot(super::GLITTER)),
    when(
        "softness",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "orientation_jitter",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "pitch_jitter",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when("tint_pass", "tint", EnabledCond::ChoiceIs(3)),
    when("tint_edge", "tint", EnabledCond::ChoiceIs(3)),
    when("tint_width", "tint", EnabledCond::ChoiceIs(3)),
    when("tint_floor", "tint", EnabledCond::ChoiceIs(3)),
    when("relief_source", "height_map", EnabledCond::LayerUnset),
    when("relief_scale", "height_map", EnabledCond::LayerUnset),
    when("relief", "relief_source", EnabledCond::ChoiceIsNot(BEVEL)),
    when(
        "relief_scale",
        "relief_source",
        EnabledCond::ChoiceIsNot(BEVEL),
    ),
    when(
        "relief_invert",
        "relief_source",
        EnabledCond::ChoiceIsNot(BEVEL),
    ),
    when("bevel_width", "relief_source", EnabledCond::ChoiceIs(BEVEL)),
    when(
        "bevel_height",
        "relief_source",
        EnabledCond::ChoiceIs(BEVEL),
    ),
    when(
        "bevel_profile",
        "relief_source",
        EnabledCond::ChoiceIs(BEVEL),
    ),
    when(
        "bevel_softness",
        "relief_source",
        EnabledCond::ChoiceIs(BEVEL),
    ),
    when("bevel_width", "height_map", EnabledCond::LayerUnset),
    when("bevel_height", "height_map", EnabledCond::LayerUnset),
    when("bevel_profile", "height_map", EnabledCond::LayerUnset),
    when("bevel_softness", "height_map", EnabledCond::LayerUnset),
    when("line_pitch", "lines", EnabledCond::ChoiceIsNot(0)),
    when("line_height", "lines", EnabledCond::ChoiceIsNot(0)),
    when("height_map_placement", "height_map", EnabledCond::LayerSet),
    when("height_map_scale", "height_map", EnabledCond::LayerSet),
    when("height_map_wrap", "height_map", EnabledCond::LayerSet),
    when("foil_mask_placement", "foil_mask", EnabledCond::LayerSet),
    when(
        "direction_map_placement",
        "direction_map",
        EnabledCond::LayerSet,
    ),
    when("direction_mode", "direction_map", EnabledCond::LayerSet),
    when("pitch_map_placement", "pitch_map", EnabledCond::LayerSet),
    when("pitch_white", "pitch_map", EnabledCond::LayerSet),
    when("environment_amount", "environment", EnabledCond::LayerSet),
    when("direction_map", "pattern", EnabledCond::ChoiceIs(FROM_MAP)),
    when(
        "direction_map_placement",
        "pattern",
        EnabledCond::ChoiceIs(FROM_MAP),
    ),
    when("direction_mode", "pattern", EnabledCond::ChoiceIs(FROM_MAP)),
    when(
        "direction_map",
        "material",
        EnabledCond::ChoiceIsNot(super::VORTEX),
    ),
    when(
        "direction_map_placement",
        "material",
        EnabledCond::ChoiceIsNot(super::VORTEX),
    ),
    when(
        "direction_mode",
        "material",
        EnabledCond::ChoiceIsNot(super::VORTEX),
    ),
    when(
        "direction_map",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "direction_map_placement",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "direction_mode",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "direction_map",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "direction_map_placement",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "direction_mode",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "pitch_map",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "pitch_map_placement",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "pitch_white",
        "material",
        EnabledCond::ChoiceIsNot(super::SHATTER),
    ),
    when(
        "pitch_map",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "pitch_map_placement",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
    when(
        "pitch_white",
        "material",
        EnabledCond::ChoiceIsNot(super::GLITTER),
    ),
];
