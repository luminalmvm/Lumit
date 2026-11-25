//! Vector blur: each pixel streaks along a direction read from a map.
//!
//! The map is the layer on the Matte row, or this layer's own picture when the
//! row is unset. Its vectors come one of two ways. **Map slope** follows the
//! contours of one channel, the way AE's CC Vector Blur does: streaks run along
//! the edges of light and dark and are longest where the slope is steepest.
//! **Map channels** reads two channels as a push on each axis, mid-grey being
//! still, which is the motion vector pass a 3D package writes out.
//!
//! There is no CPU reference through the single-buffer dispatcher, which
//! carries no second picture, so `apply_cpu` keeps its identity default. The
//! oracle is [`crate::fx::cpu::vector_blur`], exercised from the lumit-gpu test.

use crate::fx::{
    cpu, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, CHANNEL_OPTIONS,
    EDGE_OPTIONS, MAP_PLACEMENT_OPTIONS,
};
use lumit_fx_macros::Effect;

pub const VECTORS_FROM_OPTIONS: &[&str] = &["Map slope", "Map channels"];

pub const STREAK_OPTIONS: &[&str] = &["Both ways", "Forwards", "Backwards"];

pub const VECTOR_BLUR_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "slope_channel",
        on: "vectors_from",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "horizontal_channel",
        on: "vectors_from",
        cond: EnabledCond::ChoiceIs(1),
    },
    EnabledWhen {
        param: "vertical_channel",
        on: "vectors_from",
        cond: EnabledCond::ChoiceIs(1),
    },
    EnabledWhen {
        param: "matte_placement",
        on: "matte",
        cond: EnabledCond::LayerSet,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "vector_blur",
    label = "Vector blur",
    version = 1,
    category = BlurSharpen,
    cost = Heavy,
    // Length is open above, so it can't be padded statically.
    roi = FullFrame,
    premultiplied = true,
    matte = (
        "matte",
        "is the vector map: its slope or its channels say which way each pixel \
         streaks and how far, and with none picked the layer's own picture is used",
    ),
    matte_channel = false,
    enabled_when = VECTOR_BLUR_ENABLED_WHEN,
)]
pub struct VectorBlur {
    #[choice(label = "Vectors from", options = *VECTORS_FROM_OPTIONS, default = 0)]
    pub vectors_from: u32,

    #[choice(label = "Slope channel", options = *CHANNEL_OPTIONS, default = 0)]
    pub slope_channel: u32,

    /// The channel that pushes sideways, mid-grey being still. Red, as a
    /// motion vector pass has it.
    #[choice(label = "Horizontal channel", options = *CHANNEL_OPTIONS, default = 2)]
    pub horizontal_channel: u32,

    #[choice(label = "Vertical channel", options = *CHANNEL_OPTIONS, default = 3)]
    pub vertical_channel: u32,

    /// The longest streak, px@comp, reached where the map is steepest or its
    /// push is strongest.
    #[slider(min = 0.0, max = 500.0, default = 40.0, hard_min = 0.0, unit = Px)]
    pub length: f32,

    /// Turns every vector. 90° streaks across the contours instead of along
    /// them.
    #[slider(
        label = "Angle offset",
        min = -180.0,
        max = 180.0,
        default = 0.0,
        hard_min = -3600.0,
        hard_max = 3600.0,
        unit = Degrees
    )]
    pub angle_offset: f32,

    #[toggle(label = "Constant length", default = false)]
    pub constant_length: bool,

    /// On bends each streak along the map as it goes. Off keeps it straight,
    /// pointing the way the map points at its own pixel.
    #[toggle(label = "Follow the vectors", default = true)]
    pub follow_vectors: bool,

    /// Both ways centres the streak on its pixel. Forwards trails it along the
    /// vectors, Backwards against them.
    #[choice(label = "Direction", options = *STREAK_OPTIONS, default = 0)]
    pub direction: u32,

    /// Per cent: how much the streak fades towards its end. 0 is even all the
    /// way.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub falloff: f32,

    /// px@comp: blurs the map before its vectors are read, which smooths
    /// noisy vectors and widens the slope round hard edges.
    #[slider(
        label = "Map softness",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub map_softness: f32,

    #[choice(label = "Map placement", options = *MAP_PLACEMENT_OPTIONS, default = 0)]
    pub matte_placement: u32,

    #[choice(label = "Edges", options = *EDGE_OPTIONS, default = 1)]
    pub edge: u32,

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

impl VectorBlur {
    /// The bundle both kernels read. The Matte row's Invert rides beside the
    /// layer binding, as Displacement map's does.
    #[must_use]
    pub fn packed(self) -> cpu::VectorBlurParams {
        let last = CHANNEL_OPTIONS.len() as u32 - 1;
        let length = self.length.max(0.0);
        let follow = self.follow_vectors;
        cpu::VectorBlurParams {
            from_channels: self.vectors_from == 1,
            channels: [
                self.slope_channel.min(last),
                self.horizontal_channel.min(last),
                self.vertical_channel.min(last),
            ],
            length,
            turn: crate::fx::maths::rgb_split_offset(1.0, self.angle_offset),
            constant_length: self.constant_length,
            follow,
            direction: self.direction.min(2),
            falloff: (self.falloff / 100.0).clamp(0.0, 1.0),
            softness: self.map_softness.max(0.0),
            edge: self.edge.min(2),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            taps: cpu::vector_blur_taps(length, follow),
        }
    }
}

pub struct VectorBlurDef;

impl EffectDef for VectorBlurDef {
    fn schema(&self) -> &'static EffectSchema {
        &<VectorBlur as EffectMetadata>::SCHEMA
    }
}
