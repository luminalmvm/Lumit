//! Tracking a grade window: a Correction's window slot follows what it sits
//! on, forwards or backwards from the playhead, or one frame at a time.
//!
//! # In plain terms
//!
//! Work out which comp frames are being tracked, then hand them to the
//! analysis thread (`lumit_render::window_track`). As it goes, each step is sent down the
//! status stream. When it stops, the window's centre, size and angle are keyed
//! on every frame it followed, as one edit: keys already on those rows inside
//! the tracked span are replaced, and the rest stay. A stopped or lost run
//! still keys the frames it got through, since that is the part the colourist
//! watched and is usually the part they want.
//!
//! The picture followed is the one coming into the Correction, so a
//! stabiliser or a warp ahead of it on the stack is seen. It is drawn in the
//! stack's own pixels, the shot alone and unmoved, so the window's rows map
//! onto it by size alone.

use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Keyframe, SideInterp};
use lumit_core::model::{
    Document, EffectInstance, EffectValue, LayerKind, ProjectItem, TransformGroup,
};
use lumit_core::sequence::ClipSource;
use lumit_core::time::{CompTime, Rational};
use lumit_render::track::{LumaFrames, Requested};
use lumit_render::window_track::{WindowEnd, WindowJob, WindowRun};
use lumit_track::window::{WindowBox, WindowOutline};
use uuid::Uuid;

use crate::api::grade_solve::{node_input_picture, StackCut};
use crate::api::layer::{InstanceHome, LayerReference};
use crate::api::track::{failure_of, BridgeTrackFailure, BridgeTrackStage};
use crate::api::BridgeError;
use crate::frb_generated::StreamSink;

/// How far a window is tracked from the playhead: to the end or the start of
/// the shot, or onto the next or previous frame alone.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTrackRun {
    Forwards,
    Backwards,
    NextFrame,
    PreviousFrame,
}

/// How a window track is getting on, as the status stream sends it.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeWindowTrackStatus {
    /// Tracking while it runs, then Done, Cancelled or Failed.
    pub stage: BridgeTrackStage,
    /// Frames followed so far, and how many are being tracked.
    pub done: u32,
    pub total: u32,
    /// The comp frame the window was last placed on.
    pub frame: i64,
    /// The comp frame the window could not be followed onto, once lost.
    pub lost: Option<i64>,
    /// Set only at Failed, and not when the keys could not be written.
    pub failure: Option<BridgeTrackFailure>,
}

/// Where a shot's picture comes from.
enum Source {
    Footage(Uuid),
    Comp(Uuid),
}

/// Everything a window track needs once it is under way.
struct Plan {
    job: WindowJob,
    /// The comp frame of each step.
    frames: Vec<i64>,
    /// Each step's key time, on the clock the stack's keys are on.
    keys_at: Vec<Rational>,
    home: InstanceHome,
}

impl LayerReference {
    /// Track window `slot` (1 to 4) of the Correction `effect` on this layer,
    /// or on its clip `clip`, from comp frame `frame` as far as `run` says.
    /// Returns once the run has started; `status` hears each frame and then
    /// how it ended, and the keys land as one edit when it stops.
    pub fn track_grade_window(
        &self,
        clip: Option<Uuid>,
        effect: Uuid,
        slot: u32,
        frame: i64,
        run: BridgeTrackRun,
        status: StreamSink<BridgeWindowTrackStatus>,
    ) -> Result<(), BridgeError> {
        let Plan {
            job,
            frames,
            keys_at,
            home,
        } = self.window_plan(clip, effect, slot, frame, run)?;
        let place = move |frames: &[i64], done: usize| {
            frames
                .get(done.saturating_sub(1))
                .or(frames.first())
                .copied()
                .unwrap_or(frame)
        };
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);

        let (sink, steps) = (status.clone(), frames.clone());
        let step = move |done: usize, total: usize| {
            // A dropped report costs the bar one step and nothing else.
            let _ = sink.add(BridgeWindowTrackStatus {
                stage: BridgeTrackStage::Tracking,
                done: count(done),
                total: count(total),
                frame: place(&steps, done),
                lost: None,
                failure: None,
            });
        };
        let layer = *self;
        let finished = move |outcome: Result<WindowRun, lumit_render::track::AnalysisError>| {
            let total = count(frames.len());
            let report = match outcome {
                Err(why) => BridgeWindowTrackStatus {
                    stage: BridgeTrackStage::Failed,
                    done: 0,
                    total,
                    frame,
                    lost: None,
                    failure: Some(failure_of(&why)),
                },
                Ok(run) => {
                    let done = run.boxes.len();
                    let lost = (run.end == WindowEnd::Lost)
                        .then(|| frames.get(done).copied())
                        .flatten();
                    // The start frame alone is no track: nothing is written.
                    let wrote = if done >= 2 {
                        layer.with_instances(home, |effects| {
                            write_keys(effects, effect, slot, &keys_at, &run.boxes)
                        })
                    } else {
                        Ok(())
                    };
                    let (stage, failure) = match (wrote, run.end) {
                        (Err(_), _) => (BridgeTrackStage::Failed, None),
                        (Ok(()), WindowEnd::Lost) if done < 2 => (
                            BridgeTrackStage::Failed,
                            Some(BridgeTrackFailure::NoFeatures),
                        ),
                        (Ok(()), WindowEnd::Cancelled) => (BridgeTrackStage::Cancelled, None),
                        (Ok(()), _) => (BridgeTrackStage::Done, None),
                    };
                    BridgeWindowTrackStatus {
                        stage,
                        done: count(done),
                        total,
                        frame: place(&frames, done),
                        lost,
                        failure,
                    }
                }
            };
            let _ = status.add(report);
        };
        match lumit_render::window_track::request_window(job, step, finished) {
            Requested::Started => Ok(()),
            Requested::Busy | Requested::Refused => Err(BridgeError::AnalysisBusy),
        }
    }

    /// The frames, the start box and the picture for one window track,
    /// refusing what cannot be tracked.
    #[frb(ignore)]
    fn window_plan(
        &self,
        clip: Option<Uuid>,
        effect: Uuid,
        slot: u32,
        frame: i64,
        run: BridgeTrackRun,
    ) -> Result<Plan, BridgeError> {
        if !(1..=4).contains(&slot) {
            return Err(BridgeError::InvalidParam);
        }
        let doc = self.document()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidItem)?;
        let layer = comp
            .layers
            .iter()
            .find(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;
        let rate = comp.frame_rate;
        let offset = layer.start_offset.0;
        // The shot's comp frames, the end left out.
        let mut first = rate.frame_at(layer.in_point).max(0);
        let mut end = rate
            .frame_at(layer.out_point)
            .min(rate.frame_at(CompTime(comp.duration.0)));

        let shown = match clip {
            None => None,
            Some(id) => {
                let LayerKind::Sequence { clips } = &layer.kind else {
                    return Err(BridgeError::NotSequence);
                };
                Some(
                    clips
                        .iter()
                        .find(|c| c.id == id)
                        .ok_or(BridgeError::InvalidLayer)?,
                )
            }
        };
        let (stack, zero, source, home) = match shown {
            None => {
                let source = match layer.kind {
                    LayerKind::Footage { item } => Source::Footage(item),
                    LayerKind::Precomp { comp } => Source::Comp(comp),
                    _ => return Err(BridgeError::NotFootage),
                };
                (&layer.effects, offset, source, InstanceHome::Effects)
            }
            Some(c) => {
                // A clip's keys count from its own start.
                let zero = offset
                    .checked_add(c.place_start)
                    .map_err(|_| BridgeError::InvalidTime)?;
                let stop = zero
                    .checked_add(c.place_duration)
                    .map_err(|_| BridgeError::InvalidTime)?;
                first = first.max(rate.frame_at(CompTime(zero)));
                end = end.min(rate.frame_at(CompTime(stop)));
                let source = match c.source {
                    ClipSource::Footage(item) => Source::Footage(item),
                    ClipSource::Comp(nested) => Source::Comp(nested),
                    ClipSource::Solid(_) => return Err(BridgeError::NotFootage),
                };
                (&c.effects, zero, source, InstanceHome::Clip(c.id))
            }
        };
        let fx = stack
            .iter()
            .find(|e| e.id == effect && e.effect.match_name == CORRECTION)
            .ok_or(BridgeError::InvalidEffect)?;
        let row = |name: &str| format!("win{slot}_{name}");
        let outline = match fx.param(&row("shape")) {
            Some(EffectValue::Choice(1)) => WindowOutline::Ellipse,
            Some(EffectValue::Choice(2..=4)) => WindowOutline::Box,
            _ => return Err(BridgeError::InvalidParam),
        };

        let steps: Vec<i64> = match run {
            BridgeTrackRun::Forwards => (frame..end).collect(),
            BridgeTrackRun::Backwards => (first..=frame).rev().collect(),
            BridgeTrackRun::NextFrame => vec![frame, frame.saturating_add(1)],
            BridgeTrackRun::PreviousFrame => vec![frame, frame.saturating_sub(1)],
        };
        let nested = |id: Uuid| doc.comp(id);
        let mut frames = Vec::with_capacity(steps.len());
        let mut keys_at = Vec::with_capacity(steps.len());
        let mut times = Vec::with_capacity(steps.len());
        for n in steps {
            if n < first || n >= end {
                break;
            }
            let t = rate
                .time_of_frame(n)
                .map_err(|_| BridgeError::InvalidTime)?
                .0;
            frames.push(n);
            keys_at.push(t.checked_sub(zero).map_err(|_| BridgeError::InvalidTime)?);
            times.push(t.to_f64());
        }
        if frames.len() < 2 {
            return Err(BridgeError::InvalidTime);
        }

        let at = keys_at.first().map_or(0.0, |t| t.to_f64());
        let value = |name: &str| match fx.param(&row(name)) {
            Some(EffectValue::Float(p)) => Ok(p.value_at(at)),
            _ => Err(BridgeError::InvalidParam),
        };
        let start = WindowBox {
            centre: [value("x")?, value("y")?],
            size: [value("w")?, value("h")?],
            angle: value("angle")?,
        };

        let raster = match (clip, &source) {
            (Some(id), _) => self.clip_picture_size(id)?,
            (None, Source::Comp(id)) => {
                let inner = nested(*id).ok_or(BridgeError::InvalidItem)?;
                (f64::from(inner.width), f64::from(inner.height))
            }
            (None, Source::Footage(item)) => {
                let footage = doc
                    .items
                    .iter()
                    .find_map(|i| match i {
                        ProjectItem::Footage(f) if f.id == *item => Some(f),
                        _ => None,
                    })
                    .ok_or(BridgeError::InvalidItem)?;
                let proj = self.project()?;
                let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
                let (_, w, h) =
                    crate::api::composition::CompositionReference::footage_span_and_size(
                        &state, footage, comp,
                    );
                (w, h)
            }
        };
        let own = own_pixels(&doc, self.comp_id, self.layer_id, raster);
        let picture = NodeLuma {
            doc: Arc::new(own),
            comp: self.comp_id,
            layer: self.layer_id,
            clip,
            effect,
            frames: usize::try_from(end).unwrap_or(0),
            fps: rate.fps(),
            size: (0, 0),
            first: None,
        };
        let start_frame = frames.first().copied().unwrap_or(frame);
        let open: Box<dyn FnOnce() -> Option<Box<dyn LumaFrames>> + Send> = Box::new(move || {
            picture
                .open(start_frame)
                .map(|p| Box::new(p) as Box<dyn LumaFrames>)
        });
        Ok(Plan {
            job: WindowJob {
                effect,
                open,
                times,
                raster,
                start,
                outline,
            },
            frames,
            keys_at,
            home,
        })
    }
}

/// Stop the window track running on the Correction `effect`. The frames
/// already followed are still keyed.
#[frb(sync)]
pub fn cancel_window_track(effect: Uuid) {
    lumit_render::track::cancel(effect);
}

/// The Correction effect's match name.
const CORRECTION: &str = "correction";

/// The tracked picture's longest edge, in pixels: enough detail to follow,
/// quick enough to draw every frame.
const TRACK_EDGE: u32 = 1280;

/// `doc` with the shot drawn in its stack's own pixels: the comp the size of
/// `raster` and the layer unmoved, unparented and flat, so a pixel of the
/// picture is a pixel of the window's rows.
fn own_pixels(doc: &Document, comp: Uuid, layer: Uuid, raster: (f64, f64)) -> Document {
    let mut own = doc.clone();
    if let Some(c) = own.comp_mut(comp) {
        c.width = raster.0.round().max(1.0) as u32;
        c.height = raster.1.round().max(1.0) as u32;
        if let Some(l) = c.layers.iter_mut().find(|l| l.id == layer) {
            l.transform = TransformGroup::default();
            l.parent = None;
            l.switches.three_d = false;
            l.switches.motion_blur = false;
        }
    }
    own
}

/// The picture coming into the Correction, frame by frame, as luma: the comp
/// frames of the shot drawn by [`node_input_picture`], the stack cut just
/// before the node.
#[frb(ignore)]
struct NodeLuma {
    doc: Arc<Document>,
    comp: Uuid,
    layer: Uuid,
    clip: Option<Uuid>,
    effect: Uuid,
    frames: usize,
    fps: f64,
    size: (u32, u32),
    /// The first frame, drawn to learn the size and kept for its step.
    first: Option<(usize, Vec<f32>)>,
}

impl NodeLuma {
    /// Draw frame `n` once to learn the picture's size. `None` when it
    /// can't be drawn.
    fn open(mut self, n: i64) -> Option<Self> {
        let (luma, size) = self.draw(n)?;
        self.size = size;
        self.first = Some((usize::try_from(n).ok()?, luma));
        Some(self)
    }

    fn draw(&self, n: i64) -> Option<(Vec<f32>, (u32, u32))> {
        let cut = StackCut::Before(self.effect);
        let p = node_input_picture(
            &self.doc, self.comp, self.layer, self.clip, n, cut, TRACK_EDGE,
        )
        .ok()?;
        // Display-like values, as a decoded file gives the other trackers.
        let luma = p
            .rgba
            .chunks_exact(4)
            .map(|px| {
                let y = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
                y.max(0.0).powf(1.0 / 2.4).min(1.0)
            })
            .collect();
        Some((luma, (p.width, p.height)))
    }
}

impl LumaFrames for NodeLuma {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, self.size.0, self.size.1, self.fps)
    }

    fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
        if let Some((_, luma)) = self.first.take_if(|(at, _)| *at == n) {
            return Some(luma);
        }
        let (luma, size) = self.draw(i64::try_from(n).ok()?)?;
        (size == self.size).then_some(luma)
    }
}

/// Key slot `slot`'s centre, size and angle at `at` from `boxes`, step for
/// step, replacing any key inside the span. A row an expression drives is
/// left to it.
fn write_keys(
    effects: &mut [EffectInstance],
    effect: Uuid,
    slot: u32,
    at: &[Rational],
    boxes: &[WindowBox],
) -> Result<(), BridgeError> {
    let fx = effects
        .iter_mut()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    let steps: Vec<(Rational, &WindowBox)> = at.iter().copied().zip(boxes).collect();
    let (Some(lo), Some(hi)) = (
        steps.iter().map(|s| s.0).min(),
        steps.iter().map(|s| s.0).max(),
    ) else {
        return Ok(());
    };
    let read = |name: &str, b: &WindowBox| match name {
        "x" => b.centre[0],
        "y" => b.centre[1],
        "w" => b.size[0],
        "h" => b.size[1],
        _ => b.angle,
    };
    for name in ["x", "y", "w", "h", "angle"] {
        let id = format!("win{slot}_{name}");
        let Some(EffectValue::Float(property)) = fx
            .params
            .iter_mut()
            .find(|p| p.id == id)
            .map(|p| &mut p.value)
        else {
            continue;
        };
        let mut keys = match &property.animation {
            Animation::Expression(_) => continue,
            Animation::Static(_) => Vec::new(),
            Animation::Keyframed(keys) => keys
                .iter()
                .filter(|k| k.time < lo || k.time > hi)
                .cloned()
                .collect(),
        };
        keys.extend(steps.iter().map(|(time, b)| Keyframe {
            time: *time,
            value: read(name, b),
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }));
        keys.sort_by_key(|k| k.time);
        property.animation = Animation::Keyframed(keys);
    }
    Ok(())
}
