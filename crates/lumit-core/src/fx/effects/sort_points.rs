//! Sort distribution puts a stream's points in another order.
//!
//! The order is what Clone to points' In order, Vary points' Index pattern
//! and Connect points' Next walk, so sorting changes which copy is first,
//! which point a ramp starts on, and where a line runs next. Sort by says
//! what decides it: the points' own dice (a shuffle), where they are across
//! or down the frame, how far they are from Centre or which way round it
//! they lie, the number they carry, or their size. Reverse turns the order
//! round. Every point keeps its `id`, so a Trail below still follows it.
//! The sorted stream goes out on the Points socket, and is drawn as discs
//! unless Mix is 0. Nothing wired draws nothing.

use crate::fx::effects::vary_points::{POINTS_IN, POINTS_OUT};
use crate::fx::points::{self, PointsStream};
use crate::fx::{
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, registry::RowState, registry::Signature, schema::EffectSchema,
    schema::EnabledCond, schema::EnabledWhen,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// What the points are sorted by, in Choice order.
pub const SORT_BY_OPTIONS: &[&str] = &[
    "Shuffle",
    "Position x",
    "Position y",
    "Distance",
    "Angle",
    "Index",
    "Size",
];
const BY_SHUFFLE: u32 = 0;
const BY_X: u32 = 1;
const BY_Y: u32 = 2;
const BY_DISTANCE: u32 = 3;
const BY_ANGLE: u32 = 4;
const BY_INDEX: u32 = 5;

/// The dice a shuffle reads, apart from every other attribute's.
const SHUFFLE: u32 = 0x5048_5546;

/// The seed is read only by Shuffle. Centre, read only by Distance and Angle,
/// is greyed by [`SortPointsDef::row_state`].
pub const SORT_POINTS_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "seed",
    on: "sort_by",
    cond: EnabledCond::ChoiceIs(BY_SHUFFLE),
}];

/// Sort distribution's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "sort_points",
    label = "Sort distribution",
    version = 1,
    category = Points,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    enabled_when = SORT_POINTS_ENABLED_WHEN,
)]
pub struct SortPoints {
    /// What decides the order. Shuffle mixes the points by their own dice,
    /// the same on every frame for one Seed. Position x and Position y go left to right
    /// and top to bottom, Distance from Centre outwards, Angle clockwise
    /// round Centre from the right. Index (the number each point carries) and Size go from the smallest.
    #[choice(label = "Sort by", options = SORT_BY_OPTIONS, default = 0)]
    pub sort_by: u32,

    /// The order turned round: the last first.
    #[toggle(label = "Reverse", default = false)]
    pub reverse: bool,

    /// Where Distance and Angle are measured from, px@comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp. See [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// Which order Shuffle makes.
    #[seed]
    pub seed: u32,

    /// How soft the disc a point is drawn as is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// handed on and nothing is drawn.
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

impl SortPoints {
    /// The raster factor, since a stream off a wire arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// The stream in its new order. Points that tie keep the order they came
    /// in, so a sort by Position x of a grid keeps each column top to bottom.
    #[must_use]
    pub fn apply(self, input: &PointsStream) -> PointsStream {
        let mut out = input.clone();
        let (cx, cy) = (self.centre_x, self.centre_y);
        let key = |i: usize| -> f32 {
            let [x, y, _] = input.position.get(i).copied().unwrap_or_default();
            match self.sort_by {
                BY_SHUFFLE => {
                    points::draw(self.seed, input.id.get(i).copied().unwrap_or(i as u64), SHUFFLE)
                }
                BY_X => x,
                BY_Y => y,
                BY_DISTANCE => (x - cx).hypot(y - cy),
                // Clockwise from the right, as y runs down: 0 to a turn.
                BY_ANGLE => (y - cy).atan2(x - cx).rem_euclid(std::f32::consts::TAU),
                BY_INDEX => input.index.get(i).copied().unwrap_or(0.0),
                _ => input.size.get(i).copied().unwrap_or(0.0),
            }
        };
        let keys: Vec<f32> = (0..input.len()).map(key).collect();
        let mut order: Vec<usize> = (0..input.len()).collect();
        order.sort_by(|a, b| keys[*a].total_cmp(&keys[*b]));
        if self.reverse {
            order.reverse();
        }
        out.reorder(&order);
        out
    }
}

/// Sort distribution's behaviour, the same shape as Pick points'.
pub struct SortPointsDef;

impl EffectDef for SortPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SortPoints as EffectMetadata>::SCHEMA
    }

    /// A stream in and a stream out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(SortPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    /// Centre greys where the order takes no notice of it.
    fn row_state(&self, inst: &EffectInstance) -> RowState {
        let by = match inst.param("sort_by") {
            Some(EffectValue::Choice(v)) => *v,
            _ => BY_SHUFFLE,
        };
        RowState {
            disabled: if matches!(by, BY_DISTANCE | BY_ANGLE) {
                Vec::new()
            } else {
                vec!["centre_x", "centre_y"]
            },
            ..RowState::default()
        }
    }

    fn modify_points(
        &self,
        p: Params<'_>,
        cx: &crate::fx::registry::ModifyCx<'_>,
    ) -> Option<PointsStream> {
        Some(SortPoints::read(p).apply(cx.input()?))
    }
}
