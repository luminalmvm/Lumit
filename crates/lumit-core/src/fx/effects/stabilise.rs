//! Stabilise: the shake taken out of a shot, or put back on another layer.
//!
//! Drop it on a footage or Precomp layer and the clip is analysed on its own
//! thread straight away; when the analysis lands the picture steadies. The
//! analysis is kept beside the trackers' in the `track/` cache folder, never
//! in the project, and a frame only reads it back. Every row here is the quick
//! half (see [`crate::stabilise`]): changing one reworks the path from the
//! analysis in hand and never reads the clip again.
//!
//! Reverse is the same effect pointed the other way: on a layer composited over
//! a stabilised one, it moves that layer with the original shake, so work done
//! on a steady plate sits back in the shot.

use uuid::Uuid;

use super::camera_track::{DENSITY_DEFAULT, DENSITY_OPTIONS};
use crate::fx::{
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, schema::EffectSchema, schema::EnabledCond, schema::EnabledWhen,
    schema::ParamGroup,
};
use crate::model::{Composition, Document, EffectInstance, Layer, LayerKind};
use crate::stabilise::analysis::Filed;
use crate::stabilise::motion::{Mat3, IDENTITY};
use crate::stabilise::plan::{plan_for, FrameMap, Plan, PlanRows};
use crate::stabilise::points::ShownPoint;
use crate::stabilise::resolve::{forward, resolve, reversed};
use crate::stabilise::rows::{choice, Reader};
use crate::stabilise::shown::Shown;
use crate::stabilise::warp::WarpParams;
use lumit_fx_macros::Effect;
use std::sync::Arc;

/// One spelling, because the analysis job, the bridge and the Viewer compare
/// against it.
pub const MATCH_NAME: &str = "stabilise";

/// Mode's option labels: take the shake out, or put another layer's back.
pub const MODE_OPTIONS: &[&str] = &["Stabilise", "Reverse"];
/// The Mode index meaning Reverse.
pub const MODE_REVERSE: u32 = 1;

/// Result's option labels.
pub const RESULT_OPTIONS: &[&str] = &["Smooth motion", "No motion"];
/// The Result index meaning No motion.
pub const RESULT_NO_MOTION: u32 = 1;

/// Method's option labels: what the frame's motion is fitted as.
pub const METHOD_OPTIONS: &[&str] = &["Position", "Scale and rotation", "Perspective"];
/// The default Method: a slide, a turn and a growth. On ordinary shake it is
/// steadier than the four-corner fit, and on hard footage it holds on longer.
pub const METHOD_DEFAULT: u32 = 1;

/// Edges' option labels.
pub const EDGES_OPTIONS: &[&str] = &["Show", "Crop", "Crop and scale", "Fill"];
/// The default Edges: scale up until the moving edges are out of frame.
pub const EDGES_DEFAULT: u32 = 2;
/// The Edges index meaning Fill: the edges filled from the frames either side.
pub const EDGES_FILL: u32 = 3;

/// Rolling shutter's option labels. Off is the default: the read time is
/// only measured well enough to correct when a clip shows a lot of it.
pub const ROLLING_SHUTTER_OPTIONS: &[&str] = &["Off", "Reduce"];

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    visible_when: Option<(&'static str, &'static [u32])>,
    collapsed: bool,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// Reverse shows the layer it reverses; Stabilise shows everything else.
pub const STABILISE_GROUPS: &[ParamGroup] = &[
    group("", &["stabilised"], Some(("mode", &[MODE_REVERSE])), false),
    group(
        "",
        &[
            "result",
            "smoothness",
            "reference",
            "use_frame",
            "method",
            "keep_scale",
            "rolling_shutter",
        ],
        Some(("mode", &[0])),
        false,
    ),
    group(
        "Edges",
        &[
            "edges",
            "max_scale",
            "margin",
            "steady_crop",
            "extra_scale",
            "fill_from",
            "fill_feather",
        ],
        Some(("mode", &[0])),
        false,
    ),
    group(
        "Trim",
        &["trim_left", "trim_top", "trim_right", "trim_bottom"],
        Some(("mode", &[0])),
        true,
    ),
    group(
        "Analysis",
        &[
            "density",
            "use_masks",
            "frames",
            "show_points",
            "point_size",
        ],
        Some(("mode", &[0])),
        true,
    ),
];

/// Rows that grey out while another says they do nothing.
pub const STABILISE_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "analyse",
        on: "mode",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "cancel",
        on: "mode",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "smoothness",
        on: "result",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "reference",
        on: "result",
        cond: EnabledCond::ChoiceIs(RESULT_NO_MOTION),
    },
    EnabledWhen {
        param: "use_frame",
        on: "result",
        cond: EnabledCond::ChoiceIs(RESULT_NO_MOTION),
    },
    EnabledWhen {
        param: "max_scale",
        on: "edges",
        cond: EnabledCond::ChoiceIs(2),
    },
    EnabledWhen {
        param: "margin",
        on: "edges",
        cond: EnabledCond::ChoiceIs(2),
    },
    EnabledWhen {
        param: "steady_crop",
        on: "edges",
        cond: EnabledCond::ChoiceIsNot(0),
    },
    EnabledWhen {
        param: "steady_crop",
        on: "edges",
        cond: EnabledCond::ChoiceIsNot(EDGES_FILL),
    },
    filled("fill_from"),
    filled("fill_feather"),
    filled("trim_left"),
    filled("trim_top"),
    filled("trim_right"),
    filled("trim_bottom"),
    EnabledWhen {
        param: "point_size",
        on: "show_points",
        cond: EnabledCond::BoolIs(true),
    },
];

/// A row that only does anything while the edges are filled.
const fn filled(param: &'static str) -> EnabledWhen {
    EnabledWhen {
        param,
        on: "edges",
        cond: EnabledCond::ChoiceIs(EDGES_FILL),
    }
}

/// Stabilise's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "stabilise",
    label = "Stabilise",
    version = 1,
    category = Distortion,
    // One matrix multiply, one divide and a bicubic tap a pixel, and one more
    // pass a neighbour for Fill.
    cost = Cheap,
    // Fill reads the frames either side; `frames_needed` says which, and
    // nothing at all unless the edges are filled.
    temporal = &[0, -1, 1],
    // A frame can be moved anywhere, so any input pixel can land anywhere.
    roi = FullFrame,
    premultiplied = true,
    // A per-pixel strength would tear the picture between where it was and
    // where it is moved to.
    matte = false,
    groups = STABILISE_GROUPS,
    enabled_when = STABILISE_ENABLED_WHEN,
)]
pub struct Stabilise {
    /// Analyse the clip again. A fresh effect starts one by itself.
    #[action(label = "Analyse")]
    pub analyse: (),
    /// Stop a running analysis.
    #[action(label = "Cancel")]
    pub cancel: (),

    /// Take the shake out, or put another layer's shake back on this one.
    #[choice(label = "Mode", options = MODE_OPTIONS, default = 0)]
    pub mode: u32,

    /// Reverse: the layer whose stabilisation is put back. A Precomp layer
    /// means the stabilised layer inside it, which is what this row names on a
    /// stabilised precomp's own layer.
    #[layer(label = "Stabilised layer", self_default = true)]
    pub stabilised: bool,

    /// Keep the camera's own movement and take out the shake, or hold the shot
    /// still on the reference frame.
    #[choice(label = "Result", options = RESULT_OPTIONS, default = 0)]
    pub result: u32,

    /// How much of the camera's movement is smoothed away. Nought leaves the
    /// shot as it was; over 100 smooths over seconds and crops more.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 300.0,
        unit = Percent
    )]
    pub smoothness: f32,

    /// No motion: the moment of the layer every frame is held to.
    #[slider(label = "Reference", min = 0.0, max = 60.0, default = 0.0, hard_min = 0.0, unit = Seconds)]
    pub reference: f32,

    /// Hold the shot on the frame the playhead is on.
    #[action(label = "Use this frame")]
    pub use_frame: (),

    /// What the frame's motion is fitted as.
    #[choice(label = "Method", options = METHOD_OPTIONS, default = METHOD_DEFAULT)]
    pub method: u32,

    /// Leave the picture's growth alone, so walking towards something is not
    /// read as a zoom to undo.
    #[toggle(label = "Keep scale", default = false)]
    pub keep_scale: bool,

    /// Take out most of the skew and wobble a sensor read row by row gives a
    /// moving camera. Measured by the analysis; nothing moves when it saw none.
    #[choice(label = "Rolling shutter", options = ROLLING_SHUTTER_OPTIONS, default = 0)]
    pub rolling_shutter: u32,

    /// What the edges dragged into view become.
    #[choice(label = "Edges", options = EDGES_OPTIONS, default = EDGES_DEFAULT)]
    pub edges: u32,

    /// The most the picture is scaled up to push the edges out of frame.
    #[slider(
        label = "Maximum scale",
        min = 100.0,
        max = 300.0,
        default = 150.0,
        hard_min = 100.0,
        hard_max = 1000.0,
        unit = Percent
    )]
    pub max_scale: f32,

    /// A share of the frame on each side that may stay empty, for a picture
    /// that will be framed tighter later.
    #[slider(
        label = "Safe margin",
        min = 0.0,
        max = 25.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 45.0,
        unit = Percent
    )]
    pub margin: f32,

    /// How still the crop holds. At 100, the default, one crop serves the
    /// whole shot; lower lets it ease with the shot, so less is cropped.
    #[slider(
        label = "Steady crop",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub steady_crop: f32,

    /// Scale on top, without a second resample.
    #[slider(
        label = "Extra scale",
        min = 50.0,
        max = 200.0,
        default = 100.0,
        hard_min = 1.0,
        hard_max = 1000.0,
        unit = Percent
    )]
    pub extra_scale: f32,

    /// Fill: how far before and after the frame the fill may reach. Effects
    /// above Stabilise never reach the frames it fills from.
    #[slider(
        label = "Fill from",
        min = 0.1,
        max = 3.0,
        default = 0.5,
        hard_min = 0.1,
        hard_max = 10.0,
        unit = Seconds
    )]
    pub fill_from: f32,

    /// Fill: how far in from its edge each frame fades into the fill.
    #[slider(label = "Fill feather", min = 0.0, max = 200.0, default = 20.0, hard_min = 0.0, unit = Px)]
    pub fill_feather: f32,

    /// Fill: pixels of every frame's own edge that are never used, for footage
    /// with a dirty edge.
    #[slider(label = "Trim left", min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub trim_left: f32,
    #[slider(label = "Trim top", min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub trim_top: f32,
    #[slider(label = "Trim right", min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub trim_right: f32,
    #[slider(label = "Trim bottom", min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub trim_bottom: f32,

    /// How many features the analysis follows. The trackers' row.
    #[choice(options = DENSITY_OPTIONS, default = DENSITY_DEFAULT, label = "Feature density")]
    pub density: u32,

    /// Whether the layer's masks are left out of the analysis.
    #[toggle(default = true, label = "Use masks")]
    pub use_masks: bool,

    /// Which frames Analyse reads - see [`crate::planes::FRAMES_OPTIONS`].
    #[choice(options = *crate::planes::FRAMES_OPTIONS, default = 0, label = "Frames")]
    pub frames: u32,

    /// Draw the followed points over the picture.
    #[toggle(default = false, label = "Show points")]
    pub show_points: bool,

    /// How big the points are drawn.
    #[slider(
        label = "Point size",
        min = 25.0,
        max = 400.0,
        default = 100.0,
        hard_min = 10.0,
        hard_max = 1000.0,
        unit = Percent
    )]
    pub point_size: f32,

    /// How much of the correction is used. Key it to nought to pin the shot
    /// to its own motion on a frame.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub amount: f32,
}

/// The instance field the user's dropped points are kept in, as an ascending
/// list of track ids. In the instance's spare map rather than as a row: it is a
/// list the panel edits, and no row could hold it or should animate it.
pub const DROPPED_KEY: &str = "stabilise_dropped";

/// The instance field naming the analysis the dropped ids belong to: its
/// points key ([`crate::stabilise::analysis::Analysis::points_key`]), in hex.
/// Ids mean something for one analysis only, so a list kept under any other
/// is read as empty.
pub const DROPPED_FOR_KEY: &str = "stabilise_dropped_for";

fn hex(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

/// The track ids `inst` has had dropped from the analysis whose points key is
/// `points`, ascending. Empty for a list made under another analysis.
#[must_use]
pub fn dropped(inst: &EffectInstance, points: &[u8; 32]) -> Vec<u32> {
    let mine = inst
        .extra
        .get(DROPPED_FOR_KEY)
        .and_then(|v| v.as_str())
        .is_some_and(|k| k == hex(points));
    if !mine {
        return Vec::new();
    }
    let mut out: Vec<u32> = inst
        .extra
        .get(DROPPED_KEY)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()))
                .collect()
        })
        .unwrap_or_default();
    out.sort_unstable();
    out.dedup();
    out
}

/// `inst` with its dropped list set to `ids`, kept against the analysis whose
/// points key is `points`.
#[must_use]
pub fn with_dropped(inst: &EffectInstance, ids: &[u32], points: &[u8; 32]) -> EffectInstance {
    let mut out = inst.clone();
    if ids.is_empty() {
        out.extra.remove(DROPPED_KEY);
        out.extra.remove(DROPPED_FOR_KEY);
    } else {
        out.extra.insert(
            DROPPED_KEY.to_string(),
            serde_json::Value::Array(ids.iter().map(|&id| serde_json::Value::from(id)).collect()),
        );
        out.extra.insert(
            DROPPED_FOR_KEY.to_string(),
            serde_json::Value::String(hex(points)),
        );
    }
    out
}

/// What a layer shows: a footage item, or a Precomp layer's nested comp.
#[must_use]
pub fn source_of(layer: &Layer) -> Option<Uuid> {
    match layer.kind {
        LayerKind::Footage { item } => Some(item),
        LayerKind::Precomp { comp } => Some(comp),
        _ => None,
    }
}

impl Stabilise {
    /// The raster factor: the frame's map is in the layer's own pixels.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");
    /// The map's three rows, output pixel to input pixel, `w` unused.
    pub const DERIVED_ROW0: ParamId = ParamId::new("derived.row0");
    pub const DERIVED_ROW1: ParamId = ParamId::new("derived.row1");
    pub const DERIVED_ROW2: ParamId = ParamId::new("derived.row2");
    /// `(active, crop scale, layer width, layer height)`: whether there is a
    /// map at all, the centred frame Crop cuts to, and the pixels the map is
    /// in.
    pub const DERIVED_FRAME: ParamId = ParamId::new("derived.frame");
}

/// Map an analysed frame's pixels to the GPU op as the four values it reads.
/// Inactive is the picture as it came.
fn push_map(
    push: &mut dyn FnMut(ParamId, Value),
    map: &Mat3,
    crop: f64,
    size: [f64; 2],
    active: bool,
) {
    let row = |r: usize| Value::Vec4([map[r][0] as f32, map[r][1] as f32, map[r][2] as f32, 0.0]);
    push(Stabilise::DERIVED_ROW0, row(0));
    push(Stabilise::DERIVED_ROW1, row(1));
    push(Stabilise::DERIVED_ROW2, row(2));
    push(
        Stabilise::DERIVED_FRAME,
        Value::Vec4([
            f32::from(u8::from(active)),
            crop as f32,
            size[0] as f32,
            size[1] as f32,
        ]),
    );
}

/// What a Stabilise draws with: the plan, the analysis and the rows the plan
/// was made from, and the rate the layer's source plays at now.
pub(crate) struct Planned {
    pub plan: Arc<Plan>,
    pub filed: Filed,
    pub rows: PlanRows,
    pub rate: f64,
}

impl Planned {
    /// The analysis's pixels, which every map is in.
    pub fn size(&self) -> [f64; 2] {
        let a = &self.filed.analysis;
        [f64::from(a.width), f64::from(a.height)]
    }
}

/// The plan the Stabilise `r` reads draws with. `None` with no analysis in
/// hand for what the layer shows.
pub(crate) fn planned(r: &Reader<'_>) -> Option<Planned> {
    let filed = crate::stabilise::analysis::filed(r.inst.id)?;
    if Some(filed.source) != source_of(r.layer) {
        return None;
    }
    let rate = crate::stabilise::shown::rate(r.doc, r.layer, &filed.analysis);
    let rows = r.plan_rows(&filed, rate);
    let plan = plan_for(r.inst.id, &filed.analysis, filed.generation, &rows);
    Some(Planned {
        plan,
        filed,
        rows,
        rate,
    })
}

/// What a read that never makes a plan finds: the answer from the plan in
/// hand, the plan being made on a worker, or no analysis for what the layer
/// shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Held<T> {
    /// The answer, from the plan in hand.
    Ready(T),
    /// The plan is being made on a worker: ask again shortly.
    Making,
    /// Nothing analysed for what the layer shows.
    Nothing,
}

impl<T> Held<T> {
    /// The answer turned into another, the other two kept.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Held<U> {
        match self {
            Held::Ready(t) => Held::Ready(f(t)),
            Held::Making => Held::Making,
            Held::Nothing => Held::Nothing,
        }
    }
}

/// [`planned`], only if the plan is already in hand. With an analysis and no
/// plan, the plan is made on a worker and this answers [`Held::Making`]: what
/// a read on the interface thread goes through.
pub(crate) fn planned_held(r: &Reader<'_>) -> Held<Planned> {
    let Some(filed) = crate::stabilise::analysis::filed(r.inst.id) else {
        return Held::Nothing;
    };
    if Some(filed.source) != source_of(r.layer) {
        return Held::Nothing;
    }
    let rate = crate::stabilise::shown::rate(r.doc, r.layer, &filed.analysis);
    let rows = r.plan_rows(&filed, rate);
    match crate::stabilise::plan::held(r.inst.id, rows.key(filed.generation)) {
        Some(plan) => Held::Ready(Planned {
            plan,
            filed,
            rows,
            rate,
        }),
        None => {
            let analysis = Arc::clone(&filed.analysis);
            crate::stabilise::plan::make_later(r.inst.id, analysis, filed.generation, rows);
            Held::Making
        }
    }
}

/// What the Stabilise `r` reads draws at layer time `lt`, before the Amount
/// and the Extra scale: the frame's map and Crop's scale, the analysed frames
/// the layer shows there, and the plan. `None` with no analysis in hand for
/// what the layer shows.
pub(crate) fn frame_at(r: &Reader<'_>, lt: f64) -> Option<(FrameMap, Shown, Planned)> {
    let p = planned(r)?;
    let s = crate::stabilise::shown::shown(r.layer, r.comp_fps(), lt, p.rate, &p.filed.analysis)?;
    let fm = p.plan.at(s)?;
    Some((fm, s, p))
}

/// What a frame `inst` draws on `layer` of `comp` at layer time `lt` reads
/// beyond its own rows, for the frame key: see
/// [`crate::stabilise::resolve::stamp`].
#[must_use]
pub fn stamp(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
    lt: f64,
) -> Option<[u8; 32]> {
    crate::stabilise::resolve::stamp(doc, comp, layer, inst, lt)
}

/// The points `inst` followed on `layer`, showing at comp time `t`, where the
/// stabilised picture draws them, with the analysis's generation: see
/// [`crate::stabilise::points::points_at`].
#[must_use]
pub fn points_at(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
    t: f64,
) -> Held<(u64, Vec<ShownPoint>)> {
    crate::stabilise::points::points_at(doc, comp, layer, inst, t)
}

/// The scale the shot's edges need on the frames the layer shows, and whether
/// that is more than the Maximum scale allows: what the status line reports.
/// Read off a plan already in hand and never made here: with none, the plan
/// is made on a worker. The rolling shutter is left out of it, a few pixels at
/// most.
#[must_use]
pub fn needed_scale(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
) -> Held<(f64, bool)> {
    planned_held(&Reader {
        doc,
        comp,
        layer,
        inst,
    })
    .map(|p| (p.plan.framed.needed, p.plan.framed.short))
}

/// The layer a Reverse `inst` on `layer` of `comp` puts the shake of back,
/// its Stabilise, and whether that layer's analysis is in hand. `None` when
/// it names nothing with a Stabilise taking the shake out.
#[must_use]
pub fn reversing<'a>(
    doc: &'a Document,
    comp: &'a Composition,
    layer: &'a Layer,
    inst: &EffectInstance,
) -> Option<(&'a Layer, &'a EffectInstance, bool)> {
    let (target, _) = reversed(doc, comp, layer, inst)?;
    let fx = forward(target)?;
    let ready = crate::stabilise::analysis::filed(fx.id)
        .is_some_and(|f| Some(f.source) == source_of(target));
    Some((target, fx, ready))
}

/// The points to drop when `picked` are dropped from `inst` on `layer`: those
/// and every point moving with them, added to what was dropped before, and
/// the points key of the analysis they belong to. Ascending. `None` with no
/// analysis in hand, which has no points to pick.
#[must_use]
pub fn dropping(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
    picked: &[u32],
) -> Option<(Vec<u32>, [u8; 32])> {
    let p = planned(&Reader {
        doc,
        comp,
        layer,
        inst,
    })?;
    let mut out = p.rows.dropped.clone();
    out.extend(crate::stabilise::drop::spread(
        &p.filed.analysis,
        &p.plan.steps,
        picked,
    ));
    out.sort_unstable();
    out.dedup();
    Some((out, p.filed.points))
}
/// Stabilise's behaviour.
pub struct StabiliseDef;

impl EffectDef for StabiliseDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Stabilise as EffectMetadata>::SCHEMA
    }

    /// The Stabilised layer row is read for that layer's stabilisation, never
    /// its picture, so nothing renders it.
    fn shape_inputs(&self) -> &'static [&'static str] {
        &["stabilised"]
    }

    /// The frame's map, worked out from the plan, with the Amount and the
    /// Extra scale on top; for Reverse, the reversed layer's map inverted.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Stabilise::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        let doc = &cx.context.document;
        let comp = cx.context.comp.and_then(|id| doc.comp(id));
        let layer = comp.and_then(|c| c.layers.iter().find(|l| Some(l.id) == cx.context.layer));
        let (Some(comp), Some(layer)) = (comp, layer) else {
            return push_map(push, &IDENTITY, 1.0, [0.0; 2], false);
        };
        let reader = Reader {
            doc,
            comp,
            layer,
            inst: cx.inst,
        };
        let Some(r) = resolve(&reader, cx.lt) else {
            // Nothing filed for what the layer shows: ask for it. It is asked
            // once; the frames drawn meanwhile show the clip as shot.
            let filed = crate::stabilise::analysis::filed(cx.inst.id);
            let takes_out = choice(cx.inst, "mode", 0) != MODE_REVERSE;
            if takes_out && filed.is_none_or(|f| Some(f.source) != source_of(layer)) {
                crate::stabilise::analysis::missing(cx.inst.id, doc, comp.id, layer.id);
            }
            return push_map(push, &IDENTITY, 1.0, [0.0; 2], false);
        };
        push_map(push, &r.map, r.crop, r.size, true);
        if let Some(maps) = &r.fill {
            crate::stabilise::fill::push(push, maps);
        }
        if let Some((motion, rt, axis)) = r.shutter {
            crate::stabilise::shutter::push(push, motion, rt, axis);
            crate::stabilise::shutter::push_neighbours(push, &r.fill_motion);
        }
    }

    /// The map's rows are lengths in the layer's pixels for their last column
    /// only, which [`warp_params`] converts itself from the raster factor, so
    /// nothing is listed here.
    fn derived_spatial(&self) -> &'static [ParamId] {
        &[]
    }

    /// The frames a Fill reads either side; nothing for any other setting.
    fn frames_needed_at(&self, inst: &EffectInstance, frame: f64, fps: f64) -> Option<Vec<i32>> {
        Some(crate::stabilise::fill::reads(inst, frame, fps))
    }

    /// The neighbour frames never reach this path, as Echo's never do, so a
    /// Fill draws the frame itself with its feathered edge and nothing round
    /// it: what the GPU op draws where no neighbour was decoded.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        if let Some(wp) = warp_params(p, w, h) {
            let s = p.float(Stabilise::DERIVED_PX_SCALE, 1.0).max(1e-6);
            let off = frame_offset(p, w, h);
            let out = match crate::stabilise::fill::fill_params(p, w, h, s, off) {
                Some(fill) => crate::stabilise::fill::lay(
                    rgba,
                    None,
                    w,
                    h,
                    &crate::stabilise::fill::Layer {
                        map: wp.map,
                        to_frame: crate::stabilise::fill::SAME_FRAME,
                        rect: fill.rect,
                        feather: fill.feather,
                        motion: wp.motion,
                        readout: wp.readout,
                    },
                ),
                None => crate::stabilise::warp::warp(rgba, w, h, &wp),
            };
            if out.len() == rgba.len() {
                rgba.copy_from_slice(&out);
            }
        }
    }
}

/// How far in the layer's frame sits on a `w` by `h` raster, from the resolved
/// rows. An effect above that grows the raster (a blur spreading past the
/// edge, Tile) leaves the frame centred on the wider picture, and the map is
/// measured from the frame's own corner. Nought on a raster the frame fills.
#[must_use]
pub fn frame_offset(p: Params<'_>, w: u32, h: u32) -> [f32; 2] {
    let frame = p.vec4(Stabilise::DERIVED_FRAME, [0.0, 1.0, 0.0, 0.0]);
    let s = p.float(Stabilise::DERIVED_PX_SCALE, 1.0).max(1e-6);
    let inset = |raster: u32, layer: f32| {
        let room = raster as f32 - (layer * s).round();
        if layer > 0.0 && room >= 2.0 {
            (0.5 * room).floor()
        } else {
            0.0
        }
    };
    [inset(w, frame[2]), inset(h, frame[3])]
}

/// The warp for a `w` by `h` raster from the resolved rows: the one expression
/// the CPU reference and the GPU op both draw with. `None` when there is no
/// map yet, which draws the picture as it came.
#[must_use]
pub fn warp_params(p: Params<'_>, w: u32, h: u32) -> Option<WarpParams> {
    let frame = p.vec4(Stabilise::DERIVED_FRAME, [0.0, 1.0, 0.0, 0.0]);
    if frame[0] == 0.0 {
        return None;
    }
    // The map is in the layer's own pixels and the raster has `s` of its own to
    // one of those, so the map is conjugated by that scale, and taken about
    // the frame's corner where an effect above grew the raster.
    let s = p.float(Stabilise::DERIVED_PX_SCALE, 1.0).max(1e-6);
    let off = frame_offset(p, w, h);
    let r0 = p.vec4(Stabilise::DERIVED_ROW0, [1.0, 0.0, 0.0, 0.0]);
    let r1 = p.vec4(Stabilise::DERIVED_ROW1, [0.0, 1.0, 0.0, 0.0]);
    let r2 = p.vec4(Stabilise::DERIVED_ROW2, [0.0, 0.0, 1.0, 0.0]);
    let map = [
        [r0[0], r0[1], r0[2] * s],
        [r1[0], r1[1], r1[2] * s],
        [r2[0] / s, r2[1] / s, r2[2]],
    ];
    // Crop's frame: centred, its scale's share of the layer's frame. With
    // nothing cropped, the whole raster, so what an effect above spread past
    // the frame is kept.
    let crop = frame[1].max(1.0);
    let (rw, rh) = (w as f32, h as f32);
    let (kw, kh) = if crop > 1.0 {
        ((rw - 2.0 * off[0]) / crop, (rh - 2.0 * off[1]) / crop)
    } else {
        (rw, rh)
    };
    let (motion, readout) =
        crate::stabilise::shutter::params(p, [frame[2], frame[3]], s, off).unwrap_or_default();
    Some(WarpParams {
        map: crate::stabilise::warp::about(map, off),
        keep: [
            0.5 * (rw - kw),
            0.5 * (rh - kh),
            0.5 * (rw + kw),
            0.5 * (rh + kh),
        ],
        motion,
        readout,
    })
}

/// `map` taken `amount` of the way from the identity, entry by entry: right for
/// the small corrections a stabiliser makes, and exactly the identity at nought.
pub(crate) fn blend(map: &Mat3, amount: f64) -> Mat3 {
    let mut out = IDENTITY;
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = IDENTITY[r][c] + (map[r][c] - IDENTITY[r][c]) * amount;
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A dropped list is kept against the analysis it was made under: under
    /// any other, as a copied effect, a layer re-analysed under other rows or
    /// a Precomp rendered again has, it reads as empty.
    #[test]
    fn a_dropped_list_from_another_analysis_reads_as_empty() {
        let inst = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        let (mine, other) = ([1u8; 32], [2u8; 32]);
        let kept = with_dropped(&inst, &[9, 4, 9], &mine);
        assert_eq!(dropped(&kept, &mine), vec![4, 9]);
        assert!(dropped(&kept, &other).is_empty());
        assert!(dropped(&with_dropped(&kept, &[], &mine), &mine).is_empty());
    }
}
