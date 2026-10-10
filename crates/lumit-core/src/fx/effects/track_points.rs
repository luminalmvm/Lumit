//! Track points follows things in the layer's footage and hands them out as
//! points.
//!
//! Press Analyse and the clip is read once, on its own thread. Features mode
//! follows corners with the tracker the Camera track uses, and Blobs mode
//! follows bright or dark regions by their centres. Every point keeps one id
//! for as long as it is followed, so a Trail or a Clone to points below stays
//! attached to it.
//!
//! What the analysis found is kept beside the Camera track's solves, in the
//! `track/` cache folder and not in the project, so a long clip does not grow
//! the project file. A frame only reads it back. Until there is an analysis
//! for the rows as they stand, the effect makes no points and draws nothing,
//! and it never analyses to draw a frame.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::camera_track::{DENSITY_DEFAULT, DENSITY_OPTIONS};
use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, Port, PortType,
    ResolveCx, Signature, Value,
};
use crate::model::{EffectInstance, Layer, LayerKind};
use lumit_fx_macros::Effect;

/// One spelling, because the analysis job and the button's doorway both
/// compare against it.
pub const MATCH_NAME: &str = "track_points";

/// The same Points output every producer declares.
const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

/// Mode's option labels, in index order.
pub const MODE_OPTIONS: &[&str] = &["Features", "Blobs"];

/// The Mode index meaning blobs.
const MODE_BLOBS: u32 = 1;

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    visible_when: Option<(&'static str, &'static [u32])>,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// Each mode shows its own rows, then what a point looks like.
pub const TRACK_POINTS_GROUPS: &[ParamGroup] = &[
    group(
        "",
        &["density", "quality", "spacing", "window"],
        Some(("mode", &[0])),
    ),
    group(
        "",
        &[
            "threshold",
            "invert",
            "min_area",
            "max_area",
            "max_distance",
        ],
        Some(("mode", &[MODE_BLOBS])),
    ),
    group("Point", &["size", "feather", "colour"], None),
];

/// Track points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "track_points",
    label = "Track points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A track may be anywhere, and the discs are drawn over the whole picture.
    roi = FullFrame,
    premultiplied = true,
    // Not seeded: the points move with the footage, and the footage's own
    // frame is already in the frame's name.
    seeded = false,
    groups = TRACK_POINTS_GROUPS,
)]
pub struct TrackPoints {
    /// What is followed: corners in the picture, or whole bright regions.
    #[choice(label = "Mode", options = MODE_OPTIONS, default = 0)]
    pub mode: u32,

    /// How many features are followed. The Camera track's row, meaning the
    /// same grid and count to the same tracker.
    #[choice(options = DENSITY_OPTIONS, default = DENSITY_DEFAULT, label = "Feature density")]
    pub density: u32,

    /// How distinct a corner has to be, as a share of the frame's best one.
    /// Lower follows more and weaker points.
    #[slider(
        min = 0.0,
        max = 20.0,
        default = 1.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub quality: f32,

    /// The least distance between two features, in the footage's own pixels.
    #[slider(min = 1.0, max = 100.0, default = 6.0, hard_min = 0.0, unit = Px)]
    pub spacing: f32,

    /// How wide the patch followed round each feature is, in the footage's
    /// own pixels. Bigger holds on better and is less exact on small detail.
    #[slider(min = 5.0, max = 51.0, default = 15.0, hard_min = 5.0, hard_max = 63.0, unit = Px)]
    pub window: f32,

    /// How bright a pixel has to be to count as part of a blob, per cent of
    /// full white.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub threshold: f32,

    /// Follow dark blobs on a light picture instead.
    #[toggle(default = false)]
    pub invert: bool,

    /// The smallest blob kept, in pixels of the footage. Keeps specks out.
    #[counter(min = 0, max = 10_000, default = 16, hard_min = 0, unit = Raw)]
    pub min_area: i32,

    /// The largest blob kept, in pixels of the footage. Keeps the background
    /// out.
    #[counter(
        min = 0,
        max = 1_000_000,
        default = 1_000_000,
        hard_min = 0,
        unit = Raw
    )]
    pub max_area: i32,

    /// How far a blob may move between two frames and still be the same
    /// blob, in the footage's own pixels.
    #[slider(min = 0.0, max = 500.0, default = 50.0, hard_min = 0.0, unit = Px)]
    pub max_distance: f32,

    /// Start the analysis. A button, not a value.
    #[action(label = "Analyse")]
    pub analyse: (),

    /// Stop a running analysis.
    #[action(label = "Cancel")]
    pub cancel: (),

    /// The diameter of the disc a point is drawn as, px@comp. A blob is drawn
    /// at its own size instead.
    #[slider(min = 0.0, max = 200.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    /// How soft that disc's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The colour a point is drawn in. Scene-linear, and values above 1 are
    /// useful under a glow.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// emitted and nothing is drawn.
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

/// The rows an analysis reads, and the footage it read them against.
///
/// Read off the stored rows at layer time zero, as the Planar track's quad
/// is: an analysis is of the whole clip, so a row keyed to change part-way
/// through has no one value to analyse with. Two of these being equal is what
/// makes an analysis still good for an instance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    /// The footage item the layer shows.
    pub source: Uuid,
    pub blobs: bool,
    pub density: u32,
    /// A share, 0..1.
    pub quality: f32,
    pub spacing: f32,
    pub window: f32,
    /// Luma, 0..1.
    pub threshold: f32,
    pub invert: bool,
    pub min_area: f32,
    pub max_area: f32,
    pub max_distance: f32,
}

impl Analysis {
    /// The analysis `inst` asks for on `layer`. `None` when the layer is not
    /// footage, which has nothing to analyse.
    #[must_use]
    pub fn of(inst: &EffectInstance, layer: &Layer) -> Option<Self> {
        let LayerKind::Footage { item } = layer.kind else {
            return None;
        };
        // The fallbacks are the schema's own defaults, for a row a
        // hand-edited project left out.
        let at = |id: &str, fallback: f32| inst.float_at(id, 0.0).map_or(fallback, |v| v as f32);
        let choice = |id: &str, fallback: u32| match inst.param(id) {
            Some(crate::model::EffectValue::Choice(v)) => *v,
            _ => fallback,
        };
        let blobs = choice("mode", 0) == MODE_BLOBS;
        // Only the rows the mode reads, so moving a row the other mode owns
        // does not throw the analysis away.
        Some(if blobs {
            Analysis {
                source: item,
                blobs,
                density: 0,
                quality: 0.0,
                spacing: 0.0,
                window: 0.0,
                threshold: at("threshold", 50.0) / 100.0,
                invert: inst.bool_of("invert").unwrap_or(false),
                min_area: at("min_area", 16.0),
                max_area: at("max_area", 1_000_000.0),
                max_distance: at("max_distance", 50.0),
            }
        } else {
            Analysis {
                source: item,
                blobs,
                density: choice("density", DENSITY_DEFAULT),
                quality: at("quality", 1.0) / 100.0,
                spacing: at("spacing", 6.0),
                window: at("window", 15.0),
                threshold: 0.0,
                invert: false,
                min_area: 0.0,
                max_area: 0.0,
                max_distance: 0.0,
            }
        })
    }
}

/// One followed point: where it was on each frame from `first` on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Followed {
    /// The id the point keeps for its whole life.
    pub id: u32,
    /// The source frame it was first seen on.
    pub first: u32,
    /// One entry per frame from `first`, with no gaps: x and y in the
    /// footage's own pixels, then the tracker's confidence 0..1 for a
    /// feature, or the area in pixels for a blob.
    pub at: Vec<[f32; 3]>,
}

/// A finished analysis, as a frame reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Baked {
    /// What it was analysed under.
    pub analysis: Analysis,
    /// The footage's own rate, which the frame numbers count at.
    pub fps: f64,
    /// How many frames the clip has.
    pub frames: u32,
    /// Every followed point, in id order.
    pub tracks: Vec<Followed>,
}

/// Every analysis in hand, by Track points instance.
///
/// It is here and not behind a trait, as the Camera track's store is, because
/// the stream is made in this crate at resolve time and the walk that makes it
/// is handed no store. The analysis job in `lumit-render` puts them in. The
/// lock is only ever held to clone or swap one `Arc`.
fn table() -> &'static RwLock<HashMap<Uuid, Arc<Baked>>> {
    static TABLE: OnceLock<RwLock<HashMap<Uuid, Arc<Baked>>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Put a finished analysis in the table under the instance it was made for.
pub fn publish(effect: Uuid, baked: Baked) {
    if let Ok(mut held) = table().write() {
        held.insert(effect, Arc::new(baked));
    }
}

/// The analysis in hand for `effect`, whatever it was analysed under.
#[must_use]
pub fn baked(effect: Uuid) -> Option<Arc<Baked>> {
    table().read().ok()?.get(&effect).cloned()
}

/// Drop every analysis `keep` does not ask for, which is what closing a
/// project does. The files in the cache folder are left alone.
pub fn retain(keep: impl Fn(&Uuid) -> bool) {
    if let Ok(mut held) = table().write() {
        held.retain(|id, _| keep(id));
    }
}

/// The analysis in hand for `inst`, if it was made under the rows and the
/// footage `inst` has now. Anything else is stale and reads as none.
#[must_use]
pub fn fresh(inst: &EffectInstance, layer: &Layer) -> Option<Arc<Baked>> {
    let baked = baked(inst.id)?;
    (Analysis::of(inst, layer)? == baked.analysis).then_some(baked)
}

impl TrackPoints {
    /// The raster factor, since the tracks and the camera are in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// Which analysed frame this moment of the layer shows, or a negative
    /// number when there is no analysis to read.
    pub const DERIVED_FRAME: ParamId = ParamId::new("derived.frame");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The points on the frame the bag names, one per track alive on it, in
    /// id order. Empty when there is no analysis.
    ///
    /// The tracker's pixels are the footage's own, which for a footage layer
    /// are the layer's px@comp, as the Planar track reads its corners. So
    /// nothing converts but the raster factor.
    #[must_use]
    pub fn stream(p: Params<'_>, effect: Uuid, projection: Projection) -> PointsStream {
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        let (Ok(frame), Some(baked)) =
            (u32::try_from(p.int(Self::DERIVED_FRAME, -1)), baked(effect))
        else {
            return out;
        };
        let me = Self::read(p);
        let scale = Self::px_scale_of(p);
        out.px_scale = scale;
        let fps = baked.fps as f32;
        let size = me.size.max(0.0);
        // Premultiplied, as every colour in the working space is.
        let a = me.colour[3];
        let colour = [me.colour[0] * a, me.colour[1] * a, me.colour[2] * a, a];
        let blobs = baked.analysis.blobs;
        for track in &baked.tracks {
            if out.len() >= points::CAP_HARD as usize {
                break;
            }
            let Some(i) = frame.checked_sub(track.first).map(|i| i as usize) else {
                continue;
            };
            let Some(&[x, y, extra]) = track.at.get(i) else {
                continue;
            };
            // Speed from the frames either side, or from the one side a track
            // has at its first and last frame.
            let before = i.checked_sub(1).and_then(|j| track.at.get(j));
            let after = track.at.get(i + 1);
            let steps = u8::from(before.is_some()) + u8::from(after.is_some());
            let per_second = if steps > 0 {
                scale * fps / f32::from(steps)
            } else {
                0.0
            };
            let here = [x, y, extra];
            let (from, to) = (
                before.copied().unwrap_or(here),
                after.copied().unwrap_or(here),
            );
            out.position.push([x * scale, y * scale, 0.0]);
            out.speed.push([
                (to[0] - from[0]) * per_second,
                (to[1] - from[1]) * per_second,
                0.0,
            ]);
            out.age.push(i as f32 / fps);
            // The frames it is seen on, so a point on one frame still has a
            // life to divide by.
            out.life.push(track.at.len() as f32 / fps);
            out.size.push(if blobs {
                // The diameter of a disc of the blob's area.
                2.0 * (extra.max(0.0) / std::f32::consts::PI).sqrt() * scale
            } else {
                size
            });
            out.rotation.push(0.0);
            out.colour.push(colour);
            out.id.push(u64::from(track.id));
            // A blob has no confidence, so its number stays its place in the
            // stream.
            if !blobs {
                out.index.push(extra);
            }
            // The same under a name: how sure the tracker is of a feature,
            // or a blob's area in its own px.
            let name = if blobs { "area" } else { "confidence" };
            if let Some(last) = out.named_mut(name).and_then(|c| c.last_mut()) {
                *last = extra;
            }
        }
        out
    }

    /// How the stream is drawn: a feathered disc per point, and the Mix.
    #[must_use]
    pub fn draw_style(self) -> DrawStyle {
        DrawStyle {
            mode: RenderMode::Disc,
            feather: (self.feather / 100.0).clamp(0.0, 1.0),
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Track points' behaviour. No CPU reference through the trait, as with the
/// rest of the family: the tracks and the camera aren't in the bag.
pub struct TrackPointsDef;

impl EffectDef for TrackPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TrackPoints as EffectMetadata>::SCHEMA
    }

    /// The picture and the data, as every producer declares it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    /// The raster factor, and which analysed frame the layer is showing. The
    /// frame is found the way the render picks which frame to decode, so a
    /// retimed layer's points stay on its picture. Past either end of the
    /// clip the nearest frame is held.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(TrackPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        let frame = || {
            let comp = cx.context.document.comp(cx.context.comp?)?;
            let layer = comp
                .layers
                .iter()
                .find(|l| Some(l.id) == cx.context.layer)?;
            let baked = fresh(cx.inst, layer)?;
            let frame = (layer.source_time_at(cx.lt) * baked.fps).round();
            let last = i32::try_from(baked.frames.checked_sub(1)?).unwrap_or(i32::MAX);
            #[allow(clippy::cast_possible_truncation)]
            frame
                .is_finite()
                .then(|| (frame.clamp(0.0, f64::from(last))) as i32)
        };
        push(
            TrackPoints::DERIVED_FRAME,
            Value::Int(frame().unwrap_or(-1)),
        );
    }
}
