//! Hologram's groups and the rules that grey its rows.

use crate::fx::schema::{EnabledCond, EnabledWhen, ParamGroup};

/// Shown only while Lighting is Custom.
const CUSTOM_RIG: Option<(&str, &[u32])> = Some(("lighting", &[8]));

/// Shown only for a stereogram.
const STEREOGRAM_ONLY: Option<(&str, &[u32])> = Some(("recording", &[super::STEREOGRAM]));

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

/// The film, what it recorded and how it replays, then the sheet, lights,
/// surface and output it shares with Holographic foil.
pub const HOLOGRAM_GROUPS: &[ParamGroup] = &[
    group("Film", &["hologram_type", "efficiency", "glare"], false),
    group("Subject", &["subject_colour", "subject_depth"], false),
    group(
        "Depth",
        &[
            "depth_map",
            "depth_map_placement",
            "depth_scale",
            "depth_invert",
        ],
        false,
    ),
    group(
        "Backdrop",
        &[
            "backdrop",
            "backdrop_depth",
            "backdrop_layer",
            "backdrop_layer_placement",
            "film_depth",
            "pattern_size",
            "line_width",
            "pitch",
            "pattern_strength",
        ],
        true,
    ),
    group(
        "Replay",
        &[
            "colour",
            "reference_x",
            "reference_y",
            "bandwidth",
            "selectivity",
            "slit",
        ],
        false,
    ),
    group("Recording", &["recording", "speckle"], true),
    ParamGroup {
        label: "Stereogram",
        params: &["views", "view_blend", "view_zone"],
        // Shut like Recording, so it is drawn straight after the choice that
        // shows it rather than up among the open groups.
        collapsed: true,
        visible_when: STEREOGRAM_ONLY,
        visible_when_lens_elements: None,
    },
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
        "Lights",
        &[
            "lighting",
            "rig_rotation",
            "environment",
            "environment_amount",
        ],
        false,
    ),
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
            "waviness",
            "orange_peel",
            "lines",
            "line_pitch",
            "line_height",
            "bow",
            "bow_axis",
            "wear",
            "height_map",
            "height_map_placement",
            "height_map_scale",
            "height_map_wrap",
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

/// Bandwidth and selectivity belong to a reflection hologram, the slit to a
/// rainbow one. The grid, stars and Backdrop layer sit at Backdrop depth; the
/// line patterns at Pattern depth with their own rows. Each layer row's own
/// rows wait for its layer.
pub const HOLOGRAM_ENABLED_WHEN: &[EnabledWhen] = &[
    when(
        "bandwidth",
        "hologram_type",
        EnabledCond::ChoiceIsNot(super::RAINBOW),
    ),
    when(
        "selectivity",
        "hologram_type",
        EnabledCond::ChoiceIsNot(super::RAINBOW),
    ),
    when(
        "slit",
        "hologram_type",
        EnabledCond::ChoiceIs(super::RAINBOW),
    ),
    when("depth_map_placement", "depth_map", EnabledCond::LayerSet),
    when("depth_scale", "depth_map", EnabledCond::LayerSet),
    when("depth_invert", "depth_map", EnabledCond::LayerSet),
    when(
        "backdrop_layer_placement",
        "backdrop_layer",
        EnabledCond::LayerSet,
    ),
    when("film_depth", "backdrop", EnabledCond::ChoiceIsNot(0)),
    when("film_depth", "backdrop", EnabledCond::ChoiceIsNot(1)),
    when("film_depth", "backdrop", EnabledCond::ChoiceIsNot(2)),
    when("line_width", "backdrop", EnabledCond::ChoiceIsNot(0)),
    when("line_width", "backdrop", EnabledCond::ChoiceIsNot(1)),
    when("line_width", "backdrop", EnabledCond::ChoiceIsNot(2)),
    when("pitch", "backdrop", EnabledCond::ChoiceIsNot(0)),
    when("pitch", "backdrop", EnabledCond::ChoiceIsNot(1)),
    when("pitch", "backdrop", EnabledCond::ChoiceIsNot(2)),
    when("pattern_strength", "backdrop", EnabledCond::ChoiceIsNot(0)),
    when("pattern_strength", "backdrop", EnabledCond::ChoiceIsNot(1)),
    when("pattern_strength", "backdrop", EnabledCond::ChoiceIsNot(2)),
    when("pattern_size", "backdrop", EnabledCond::ChoiceIsNot(0)),
    when("relief_source", "height_map", EnabledCond::LayerUnset),
    when("relief_scale", "height_map", EnabledCond::LayerUnset),
    when("line_pitch", "lines", EnabledCond::ChoiceIsNot(0)),
    when("line_height", "lines", EnabledCond::ChoiceIsNot(0)),
    when("height_map_placement", "height_map", EnabledCond::LayerSet),
    when("height_map_scale", "height_map", EnabledCond::LayerSet),
    when("height_map_wrap", "height_map", EnabledCond::LayerSet),
    when("environment_amount", "environment", EnabledCond::LayerSet),
];
