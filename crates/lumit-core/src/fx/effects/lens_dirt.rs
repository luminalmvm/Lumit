//! Lens dirt: dust, grit, hairs, smudges, haze, scratches and water on the
//! glass, lit by the bright parts of the picture.
//!
//! The glass sits far closer than anything the lens focuses on, so its dirt is
//! always out of focus, by an amount the f-stop sets: wide open a speck of dust
//! is a big faint disc in the iris's shape, stopped down a small bright one.
//! Dirt only shows where light reaches it, so it is drawn by a wide spread of
//! the frame's highlights. Droplets also bend the picture behind them.
//!
//! The pattern comes from the seed, sized against the frame's diagonal, so a
//! preview at any resolution shows the same dirt. It sits on curved glass,
//! crowding towards the edges as the glass turns away.

use crate::fx::{
    registry::EffectDef, registry::EffectMetadata, schema::EffectSchema, schema::EnabledCond,
    schema::EnabledWhen, schema::ParamGroup,
};
use lumit_fx_macros::Effect;

pub const LENS_DIRT_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Dirt",
        params: &[
            "dust",
            "grit",
            "hairs",
            "smudges",
            "haze",
            "scratches",
            "droplets",
            "refraction",
        ],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Iris",
        params: &["blades", "roundness", "rotation"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Light",
        params: &["threshold", "spread", "ambient"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Colour",
        params: &["colour", "tint", "tint_2", "variation"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

pub const LENS_DIRT_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "tint_2",
    on: "colour",
    cond: EnabledCond::ChoiceIs(1),
}];

/// Lens dirt's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "lens_dirt",
    label = "Lens dirt",
    version = 1,
    category = Stylise,
    cost = Moderate,
    // The light that reaches the glass comes from anywhere in the frame.
    roi = FullFrame,
    premultiplied = true,
    groups = LENS_DIRT_GROUPS,
    enabled_when = LENS_DIRT_ENABLED_WHEN,
)]
pub struct LensDirt {
    /// How bright the lit dirt is.
    #[slider(min = 0.0, max = 2.0, default = 0.35, hard_min = 0.0, unit = Raw)]
    pub intensity: f32,

    /// The working f-stop: how far out of focus the glass is. Wire Depth of
    /// field pro's F-stop here to follow it.
    #[slider(
        label = "F-stop",
        min = 0.95,
        max = 22.0,
        default = 2.8,
        hard_min = 0.7,
        hard_max = 32.0,
        unit = Raw
    )]
    pub fstop: f32,

    /// Where the lens is focused, metres. The glass sits a few centimetres in
    /// front of the lens, so focusing close brings its dirt a little sharper.
    /// Wire Depth of field pro's Focus distance here to follow it.
    #[slider(
        label = "Focus distance (m)",
        min = 0.3,
        max = 100.0,
        default = 100.0,
        hard_min = 0.05,
        unit = Raw
    )]
    pub focus_m: f32,

    /// The pattern's size, per cent.
    #[slider(
        label = "Dirt scale",
        min = 25.0,
        max = 400.0,
        default = 100.0,
        hard_min = 5.0,
        hard_max = 2000.0,
        unit = Percent
    )]
    pub scale: f32,

    /// How far the glass bulges: 0 flat, 1 a half sphere, up to 2 a fisheye's
    /// front glass, below 0 dished. Wire Depth of field pro's Glass curvature
    /// here to follow its lens.
    #[slider(
        label = "Glass curvature",
        min = -1.0,
        max = 1.0,
        default = 0.3,
        hard_min = -2.0,
        hard_max = 2.0,
        unit = Raw
    )]
    pub curvature: f32,

    // ---- Dirt ----
    /// Specks on the lens's surfaces, out of focus into rimmed discs of many
    /// sizes.
    #[slider(min = 0.0, max = 1.0, default = 0.35, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub dust: f32,

    /// Fine sharp specks.
    #[slider(min = 0.0, max = 1.0, default = 0.2, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub grit: f32,

    /// Curling fibres.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub hairs: f32,

    /// Greasy streaks and fingerprints.
    #[slider(min = 0.0, max = 1.0, default = 0.5, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub smudges: f32,

    /// A thin film over the whole glass.
    #[slider(min = 0.0, max = 1.0, default = 0.2, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub haze: f32,

    #[slider(min = 0.0, max = 1.0, default = 0.15, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub scratches: f32,

    /// Water on the glass.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub droplets: f32,

    /// How hard the droplets bend the picture behind them.
    #[slider(min = 0.0, max = 1.0, default = 0.5, hard_min = 0.0, hard_max = 2.0, unit = Raw)]
    pub refraction: f32,

    // ---- Iris ----
    #[counter(min = 3, max = 16, default = 9, hard_min = 3, hard_max = 16, unit = Raw)]
    pub blades: i32,

    /// 0 is the blade polygon, 1 a circle.
    #[slider(min = 0.0, max = 1.0, default = 0.6, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub roundness: f32,

    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    // ---- Light ----
    /// Brightness above which the picture lights the dirt.
    #[slider(min = 0.0, max = 4.0, default = 0.9, hard_min = 0.0, unit = Raw)]
    pub threshold: f32,

    /// How far across the glass a light reaches, as a share of the frame.
    #[slider(min = 0.0, max = 1.0, default = 0.35, hard_min = 0.0, hard_max = 2.0, unit = Raw)]
    pub spread: f32,

    /// Light on all of the glass, whatever the picture.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, unit = Raw)]
    pub ambient: f32,

    // ---- Colour ----
    /// Single gives the dirt the Tint. Gradient gives each speck a colour
    /// picked at random between Tint and Second tint.
    #[choice(options = ["Single", "Gradient"], default = 0)]
    pub colour: u32,

    #[colour(default = [1.0, 1.0, 1.0, 1.0], min = 0.0, max = 4.0)]
    pub tint: [f32; 4],

    #[colour(label = "Second tint", default = [0.75, 0.8, 1.2, 1.0], min = 0.0, max = 4.0)]
    pub tint_2: [f32; 4],

    /// How far each speck's hue and brightness stray either side of its
    /// colour.
    #[slider(
        label = "Colour variation",
        min = 0.0,
        max = 1.0,
        default = 0.15,
        hard_min = 0.0,
        hard_max = 1.0,
        unit = Raw
    )]
    pub variation: f32,

    /// Final draws the picture with its dirt. Dirt alone draws the lit dirt
    /// over black, to lay over something else.
    #[choice(options = ["Final", "Dirt alone"], default = 0)]
    pub display: u32,

    #[seed]
    pub seed: u32,

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

impl LensDirt {
    /// How far out of focus the glass is, in thousandths of the frame's
    /// diagonal: about 1.2% of it at f/2.8 focused far off, halving with each
    /// two stops.
    pub fn defocus(&self) -> f32 {
        // Glass about 5 cm in front of the lens, out of focus by how far its
        // image lands from the focused one.
        let near = 1.0 - (0.05 / self.focus_m.max(0.06)).min(0.9);
        12.0 * 2.8 / self.fstop.max(0.5) * near
    }
}

pub struct LensDirtDef;

impl EffectDef for LensDirtDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LensDirt as EffectMetadata>::SCHEMA
    }

    fn view(&self) -> Option<&'static str> {
        Some("display")
    }
}
