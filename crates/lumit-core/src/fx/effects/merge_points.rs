//! Merge points joins up to four streams into one.
//!
//! Every point of every wired input goes out on the Points socket, and is
//! drawn as a disc unless Mix is 0. An input with nothing wired adds nothing.
//! Each point's `id` is made from its old one and which input it came in on,
//! so no two points share one and a Trail below still follows each of them.
//! The number each point carries says which input it came in on, and a merge
//! of merges can keep the numbers of the ones before it apart.

use crate::fx::effects::vary_points::{POINTS_OUT, POINTS_PORT};
use crate::fx::points::{self, PointsStream};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup, ParamId, Params,
    Port, PortType, ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The four streams it takes in.
const MERGE_IN: &[Port] = &[
    Port::new(POINTS_PORT, "Points", PortType::Points),
    Port::new("points_b", "Points B", PortType::Points),
    Port::new("points_c", "Points C", PortType::Points),
    Port::new("points_d", "Points D", PortType::Points),
];

pub const MERGE_POINTS_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "Point",
    params: &["feather"],
    collapsed: false,
    visible_when: None,
    visible_when_lens_elements: None,
}];

/// Count nested inputs has nothing to count while Number by input is off.
pub const MERGE_POINTS_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "count_nested",
    on: "number_by_input",
    cond: EnabledCond::BoolIs(true),
}];

/// Merge points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "merge_points",
    label = "Merge points",
    version = 1,
    category = Generate,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    groups = MERGE_POINTS_GROUPS,
    enabled_when = MERGE_POINTS_ENABLED_WHEN,
)]
pub struct MergePoints {
    /// The number each point carries becomes which input it came in on, 0
    /// for Points to 3 for Points D, so an effect below can tell them apart.
    /// Off, each point keeps the number it came with.
    #[toggle(label = "Number by input", default = true)]
    pub number_by_input: bool,

    /// An input whose points already carry different numbers, as a Merge
    /// points above hands them on, keeps them apart. Each input's numbers
    /// count on from the input before, so a merge of two merged streams and
    /// a third numbers them 0, 1 and 2, not 0, 0 and 1.
    #[toggle(label = "Count nested inputs", default = false)]
    pub count_nested: bool,

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

impl MergePoints {
    /// The raster factor, since a stream off a wire arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// Every input's points in one stream, in `id` order. The camera is the
    /// first wired input's, and every input on one layer has the same one.
    #[must_use]
    pub fn apply(self, inputs: &[&PointsStream]) -> PointsStream {
        let mut out = PointsStream::default();
        if let Some(first) = inputs.iter().find(|s| !s.is_empty()) {
            out.projection = first.projection;
        }
        // The first number the next input may use, under Count nested inputs.
        let mut next = 0.0f32;
        for (k, input) in inputs.iter().take(MERGE_IN.len()).enumerate() {
            let mut part = (*input).clone();
            // Four inputs, so an old id times four plus the input is one no
            // other point has. Wrapping, since an id that large is nonsense
            // already and must not stop the render.
            for id in &mut part.id {
                *id = id
                    .wrapping_mul(MERGE_IN.len() as u64)
                    .wrapping_add(k as u64);
            }
            if self.number_by_input && !self.count_nested {
                part.index_mut().fill(k as f32);
            } else if self.number_by_input && !part.is_empty() {
                // The different numbers this input came with, lowest first.
                // None written is one number for the whole of it.
                let mut seen = part.index.clone();
                seen.sort_by(f32::total_cmp);
                seen.dedup_by(|a, b| a.total_cmp(b).is_eq());
                for n in part.index_mut() {
                    let place = seen.binary_search_by(|s| s.total_cmp(n)).unwrap_or(0);
                    *n = next + place as f32;
                }
                next += seen.len().max(1) as f32;
            }
            out.append(&part);
        }
        out.sort_by_id();
        // Four full streams are more than one may hold, so the newest stay.
        out.keep_newest(points::CAP_HARD as usize);
        out
    }
}

/// Merge points' behaviour.
pub struct MergePointsDef;

impl EffectDef for MergePointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MergePoints as EffectMetadata>::SCHEMA
    }

    /// Four streams in and one out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: MERGE_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(MergePoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    fn modify_points(&self, p: Params<'_>, cx: &crate::fx::ModifyCx<'_>) -> Option<PointsStream> {
        Some(MergePoints::read(p).apply(cx.inputs))
    }
}
