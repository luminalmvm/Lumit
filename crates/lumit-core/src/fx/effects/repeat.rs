//! Repeat start and Repeat end: the boxes between them run several times.
//!
//! **In plain terms.** Put a Repeat start before some boxes and a Repeat end
//! after them, and the boxes wired between the two run Times times. In
//! Feedback each time works on the picture the time before made, so a turn
//! and a shrink become a spiral. In Copies each time starts from the same
//! picture and all of them are laid over each other, the first at the bottom.
//! The start hands each time its number on Index, Count, Normalised and
//! Random, as Clone index does for a copy.
//!
//! **For** says what the times are: Times of them, one for each point of the
//! stream wired into the start's Points socket, or one for each path on its
//! Shapes socket. Each time is then handed its point's or path's place on X
//! and Y, and its Size, Rotation and Colour.
//!
//! **A node graph's own**, beside the Merge: the graph walk runs the boxes
//! between the pair once a time ([`crate::comp_graph::CompGraph::repeats`]),
//! so neither box has a kernel. A start with no end, or an end with no start,
//! hands its picture on.

use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, Port, PortType, Signature,
    CHOICE_UNGROUPED,
};
use lumit_fx_macros::Effect;

/// The port each time's number leaves the start by, counted from 0.
pub const INDEX_PORT: &str = "index";
/// The port the number of times leaves by.
pub const COUNT_PORT: &str = "count";
/// The port the number as a fraction leaves by: 0 the first time, 1 the last.
pub const NORMALISED_PORT: &str = "normalised";
/// The port each time's own roll leaves by, 0 up to 1.
pub const RANDOM_PORT: &str = "random";

/// The numbers the start hands each time, in the order they are drawn: its
/// own four, then its point's or path's.
pub const PASS_PORTS: &[Port] = &[
    Port::new(INDEX_PORT, "Index", PortType::Number),
    Port::new(COUNT_PORT, "Count", PortType::Number),
    Port::new(NORMALISED_PORT, "Normalised", PortType::Number),
    Port::new(RANDOM_PORT, "Random", PortType::Number),
    Port::new("x", "X", PortType::Number),
    Port::new("y", "Y", PortType::Number),
    Port::new("size", "Size", PortType::Number),
    Port::new("rotation", "Rotation", PortType::Number),
    Port::new("colour", "Colour", PortType::Colour),
];

/// The stream Each point runs once a point of.
pub const POINTS_PORT: &str = "points";
/// The paths Each path runs once a path of.
pub const SHAPES_PORT: &str = "shapes";

const START_INPUTS: &[Port] = &[
    Port::new(POINTS_PORT, "Points", PortType::Points),
    Port::new(SHAPES_PORT, "Shapes", PortType::Shape),
];

/// The point or path one time is handed, in the graph's own pixels. Times
/// with no point or path hand on nought, and white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Element {
    pub at: [f32; 2],
    pub size: f32,
    /// Degrees.
    pub rotation: f32,
    /// Straight scene-linear RGBA.
    pub colour: [f32; 4],
}

impl Default for Element {
    fn default() -> Self {
        Element {
            at: [0.0; 2],
            size: 0.0,
            rotation: 0.0,
            colour: [1.0; 4],
        }
    }
}

/// The dice Random rolls, kept off the numbers Clone index uses.
const RANDOM_ATTR: u32 = 51;

/// Repeat start's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "repeat_start",
    label = "Repeat start",
    version = 1,
    category = Compositing,
    cost = Trivial,
    roi = FullFrame,
    matte = false,
)]
pub struct RepeatStart {
    /// Which dice Random rolls.
    #[seed]
    pub seed: u32,
}

impl RepeatStart {
    /// What the start hands time `index` of `count` on each of [`PASS_PORTS`].
    #[must_use]
    pub fn pass(self, index: u32, count: u32) -> [f32; 4] {
        let normalised = if count > 1 {
            index as f32 / (count - 1) as f32
        } else {
            0.0
        };
        let random = crate::fx::noise::hash01(self.seed, RANDOM_ATTR, index as i32, 0, 0);
        [index as f32, count as f32, normalised, random]
    }
}

/// Repeat start's behaviour: none of its own.
pub struct RepeatStartDef;

impl EffectDef for RepeatStartDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RepeatStart as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: START_INPUTS,
            extra: PASS_PORTS,
        }
    }
}

/// Times of them.
pub const FOR_TIMES: u32 = 0;
/// One for each point on the start's Points socket.
pub const FOR_POINTS: u32 = 1;
/// One for each path on the start's Shapes socket.
pub const FOR_PATHS: u32 = 2;

/// Each time works on what the time before made.
pub const MODE_FEEDBACK: u32 = 0;
/// Each time starts from the same picture, and every time is kept.
pub const MODE_COPIES: u32 = 1;

const REPEAT_END_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "times",
        on: "for_each",
        cond: EnabledCond::ChoiceIs(FOR_TIMES),
    },
    EnabledWhen {
        param: "copies_blend",
        on: "mode",
        cond: EnabledCond::ChoiceIs(MODE_COPIES),
    },
];

/// Repeat end's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "repeat_end",
    label = "Repeat end",
    version = 1,
    category = Compositing,
    cost = Trivial,
    roi = FullFrame,
    matte = false,
    enabled_when = REPEAT_END_ENABLED_WHEN,
)]
pub struct RepeatEnd {
    /// What the times are: Times of them, or one for each point or path the
    /// start is handed.
    #[choice(
        label = "For",
        options = ["Times", "Each point", "Each path"],
        default = 0
    )]
    pub for_each: u32,

    /// How many times the boxes between the pair run. Nought hands the
    /// start's picture straight on.
    #[counter(min = 0, max = 64, default = 4, hard_min = 0, hard_max = 4096, unit = Raw)]
    pub times: i32,

    /// Feedback or Copies.
    #[choice(label = "Mode", options = ["Feedback", "Copies"], default = 0)]
    pub mode: u32,

    /// How each copy is laid over the ones before it. Spelt `copies_blend`
    /// rather than `blend`, which the schema reads as the Mix seam's own row.
    #[choice(
        label = "Blend",
        options = *::lumit_core::model::BlendMode::NAMES,
        default = 0,
        dividers_after = CHOICE_UNGROUPED
    )]
    pub copies_blend: u32,
}

/// Repeat end's behaviour: none of its own.
pub struct RepeatEndDef;

impl EffectDef for RepeatEndDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RepeatEnd as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}
