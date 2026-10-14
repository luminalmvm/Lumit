//! Depth of field pro: depth of field from a real lens.
//!
//! The lens is one of the bundled prescriptions, or one kept in the effect
//! (see [`crate::fx::lens`]). [`crate::fx::lens_blur`] traces it into a table
//! of out-of-focus shapes. The frame reads a depth from the matte, turns it into
//! metres and then into how far that point's image lands from the sensor, and
//! blurs each pixel with the shape the lens gives there.
//!
//! With no matte every pixel sits at the far distance, so the effect can be
//! dropped on footage to defocus it with a lens's character.

use crate::fx::{
    lens_blur, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup,
    CHANNEL_OPTIONS,
};
use lumit_fx_macros::Effect;

pub const DOF_PRO_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Iris",
        params: &["blades", "roundness", "rotation"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Highlights",
        params: &["threshold", "boost"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Depth map",
        params: &["depth_channel", "near_m", "far_m"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

pub const DOF_PRO_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "focus_m",
        on: "use_focus_point",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "focus_point_x",
        on: "use_focus_point",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "focus_point_y",
        on: "use_focus_point",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "use_focus_point",
        on: "depth",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "depth_channel",
        on: "depth",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "near_m",
        on: "depth",
        cond: EnabledCond::LayerSet,
    },
];

/// Depth of field pro's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "dof_pro",
    label = "Depth of field pro",
    version = 1,
    category = BlurSharpen,
    cost = Heavy,
    roi = PaddedPx(256.0), // lens_blur::MAX_SPAN_PX
    premultiplied = true,
    groups = DOF_PRO_GROUPS,
    enabled_when = DOF_PRO_ENABLED_WHEN,
    matte = (
        "depth",
        "a depth pass: 0 is the nearest distance and 1 the farthest, and \
         each pixel is blurred by how far it sits from the focus distance",
    ),
    matte_channel = false,
)]
pub struct DofPro {
    /// The depth pass, on the matte carriage as Depth of field's is. Decided
    /// by the render, so always `false` here.
    #[layer(label = "Matte", self_default = false)]
    pub depth: bool,

    #[toggle(label = "Invert", default = false)]
    pub depth_invert: bool,

    /// The bundled lens. A lens kept in the effect wins over it.
    #[choice(
        label = "Lens",
        options = crate::fx::lens_library::LENS_OPTIONS,
        default = 17
    )]
    pub lens_model: u32,

    /// Opens the lens window.
    #[action(label = "Edit lens…")]
    pub edit_lens: (),

    /// The working f-stop. It cannot open wider than the lens does.
    #[slider(
        label = "F-stop",
        min = 0.95,
        max = 22.0,
        default = 1.4,
        hard_min = 0.7,
        hard_max = 32.0,
        unit = Raw
    )]
    pub fstop: f32,

    /// Where the lens is focused, metres.
    #[slider(
        label = "Focus distance (m)",
        min = 0.3,
        max = 100.0,
        default = 3.0,
        hard_min = 0.05,
        unit = Raw
    )]
    pub focus_m: f32,

    /// Focus on whatever sits under the point instead.
    #[toggle(default = false)]
    pub use_focus_point: bool,

    #[slider(min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub focus_point_x: f32,

    #[slider(min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub focus_point_y: f32,

    /// The sensor the frame was shot on. A smaller one sees less of the lens,
    /// so the frame's edge has less of its edge character, and blurs less.
    #[choice(
        options = ["Full frame", "Super 35", "APS-C", "Micro Four Thirds"],
        default = 0
    )]
    pub sensor: u32,

    /// Scales every blur after the optics, 1 being the lens as it is.
    #[slider(
        label = "Blur size",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        hard_max = 16.0,
        unit = Raw
    )]
    pub size: f32,

    /// How much the glass splits colour: 0 none, 1 as designed.
    #[slider(
        label = "Colour fringing",
        min = 0.0,
        max = 3.0,
        default = 1.0,
        hard_min = 0.0,
        hard_max = 10.0,
        unit = Raw
    )]
    pub fringing: f32,

    /// Smears and specks on the front glass, seen inside the out-of-focus
    /// highlights.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub glass_dirt: f32,

    // ---- Iris ----
    #[counter(min = 3, max = 16, default = 9, hard_min = 3, hard_max = 16, unit = Raw)]
    pub blades: i32,

    /// 0 is the blade polygon, 1 a circle.
    #[slider(min = 0.0, max = 1.0, default = 0.6, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub roundness: f32,

    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    // ---- Highlights ----
    /// Brightness above which a pixel is drawn as its own bokeh shape.
    #[slider(
        label = "Highlight threshold",
        min = 0.0,
        max = 4.0,
        default = 0.9,
        hard_min = 0.0,
        unit = Raw
    )]
    pub threshold: f32,

    /// Multiplies what is above the threshold before it is blurred.
    #[slider(
        label = "Highlight boost",
        min = 0.0,
        max = 8.0,
        default = 1.0,
        hard_min = 0.0,
        hard_max = 64.0,
        unit = Raw
    )]
    pub boost: f32,

    // ---- Depth map ----
    #[choice(options = *CHANNEL_OPTIONS, default = 0)]
    pub depth_channel: u32,

    /// The distance a depth of 0 means, metres.
    #[slider(
        label = "Nearest (m)",
        min = 0.1,
        max = 20.0,
        default = 0.5,
        hard_min = 0.01,
        unit = Raw
    )]
    pub near_m: f32,

    /// The distance a depth of 1 means, metres, and where everything sits
    /// with no matte.
    #[slider(
        label = "Farthest (m)",
        min = 1.0,
        max = 1000.0,
        default = 30.0,
        hard_min = 0.02,
        unit = Raw
    )]
    pub far_m: f32,

    /// Final draws the blur. Depth draws the metres the blur used.
    #[choice(options = ["Final", "Depth"], default = 0)]
    pub display: u32,

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

impl DofPro {
    /// The bake's inputs for a frame of `w` by `h`.
    pub fn bake_inputs(&self, w: u32, h: u32) -> lens_blur::BokehLens {
        lens_blur::BokehLens {
            lens: self.lens_model,
            fstop: self.fstop.max(0.5),
            blades: self.blades.clamp(3, 16) as u32,
            roundness: self.roundness.clamp(0.0, 1.0),
            fringing: self.fringing.max(0.0),
            glass_dirt: self.glass_dirt.clamp(0.0, 1.0),
            sensor_w_mm: self.sensor_w_mm(),
            aspect: h as f32 / w.max(1) as f32,
        }
    }

    pub fn sensor_w_mm(&self) -> f32 {
        lens_blur::SENSOR_WIDTHS_MM
            [(self.sensor as usize).min(lens_blur::SENSOR_WIDTHS_MM.len() - 1)]
    }
}

pub struct DofProDef;

impl EffectDef for DofProDef {
    fn schema(&self) -> &'static EffectSchema {
        &<DofPro as EffectMetadata>::SCHEMA
    }

    fn view(&self) -> Option<&'static str> {
        Some("display")
    }
}
