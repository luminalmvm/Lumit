//! The grade solvers: auto balance, shot match, the colour chart, and the
//! qualifier and hue curve picks.
//!
//! Each call renders the picture coming into a Correction node (its shot
//! drawn alone, the stack stopped just before the node), solves in
//! `lumit_core::grade_solve`, and hands back the rows to write. The Grade
//! panel writes them into the node as one undo step; nothing here edits the
//! document. Every call is async: it renders and reads the picture back.
//!
//! A point on the picture is in comp pixels, as the Viewer draws it.

use std::sync::Arc;

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::effect::{BridgeColour, BridgeEffectValue, BridgeScalar};
use crate::api::grade_shots::{frame_in_shot, shot_in, shot_time};
use crate::api::BridgeError;
use lumit_core::fx::effects::correction::Correction;
use lumit_core::fx::markers::MarkerContext;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::grade::groups;
use lumit_core::grade_solve::{self, chart, matching, pick, pick3d, RowValue, Rows, SolveError};
use lumit_core::model::{Document, EffectInstance, EffectValue, LayerKind};

/// The node input picture's longest edge, in pixels: enough to read tones
/// and colours, small enough to read back in a moment.
const LONG_EDGE: u32 = 512;

/// One row a solver sets, by its param id.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSolvedRow {
    pub id: String,
    pub value: BridgeEffectValue,
}

/// A chart match: the rows, and each patch's delta E 2000 in chart order
/// (dark skin first, black last), before and after.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeChartMatch {
    pub rows: Vec<BridgeSolvedRow>,
    pub before: Vec<f32>,
    pub after: Vec<f32>,
}

/// What a qualifier pick does to the ranges already there.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePickMode {
    Set,
    Add,
    Subtract,
}

/// Where a hue curve pick lands on the curve's x axis, 0..1, and how far the
/// samples spread either side.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeCurvePick {
    pub centre: f64,
    pub spread: f64,
}

/// How much of a shot's stack the node input picture keeps.
#[frb(ignore)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StackCut {
    /// Everything up to this effect. A Correction that runs in parallel or
    /// as a layer shares the input of the node its run starts from, so the
    /// cut moves up to there.
    Before(Uuid),
    /// All of it.
    Whole,
}

/// A shot's picture at one frame, scene-linear and premultiplied, four
/// floats a pixel, and how many of its pixels make one comp pixel.
#[frb(ignore)]
pub(crate) struct NodeInput {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
    pub scale: f32,
}

/// The node input picture: shot `layer` (and `clip`, on a Sequence layer)
/// drawn alone at comp `frame` (moved inside the shot), its stack cut as
/// `cut` says, longest edge about `long_edge`. Before any view or display
/// transform, clear where the shot draws nothing.
#[frb(ignore)]
pub(crate) fn node_input_picture(
    doc: &Arc<Document>,
    comp: Uuid,
    layer: Uuid,
    clip: Option<Uuid>,
    frame: i64,
    cut: StackCut,
    long_edge: u32,
) -> Result<NodeInput, BridgeError> {
    let c = doc.comp(comp).ok_or(BridgeError::InvalidComp)?;
    let (shown, clip_shown) = shot_in(c, layer, clip)?;
    let frame = frame_in_shot(c, shown, clip_shown, frame)?.max(0) as u64;
    let longest = c.width.max(c.height).max(1);
    // A node in a group's grade or a shared node: the shot's stack stops at
    // the reference that reaches it, and that list is cut instead.
    let held = match cut {
        StackCut::Before(id) => {
            let stack = clip_shown.map_or(&shown.effects, |c| &c.effects);
            groups::holder(doc, stack, id)
        }
        StackCut::Whole => None,
    };
    let own = |stack: &mut Vec<EffectInstance>| match held {
        Some((at, ..)) => {
            stack.truncate(at + 1);
            Ok(())
        }
        None => cut_stack(stack, cut),
    };

    // A patched copy, never the document: the shot alone, shown, cut.
    let mut alone = Document::clone(doc);
    let c = alone.comp_mut(comp).ok_or(BridgeError::InvalidComp)?;
    for l in &mut c.layers {
        l.switches.solo = l.id == layer;
        if l.id != layer {
            continue;
        }
        l.switches.visible = true;
        match (clip, &mut l.kind) {
            (Some(clip), LayerKind::Sequence { clips }) => {
                clips.retain(|c| c.id == clip);
                let stack = clips.first_mut().map(|c| &mut c.effects);
                own(stack.ok_or(BridgeError::InvalidItem)?)?;
                // The layer's own stack comes after every clip's.
                if cut != StackCut::Whole {
                    l.effects.clear();
                }
            }
            _ => own(&mut l.effects)?,
        }
    }
    if let Some((_, group, part)) = held {
        let g = alone
            .grade_groups
            .iter_mut()
            .find(|g| g.id == group)
            .ok_or(BridgeError::InvalidEffect)?;
        cut_stack(g.part_mut(part), cut)?;
    }
    let scale = (f64::from(long_edge.max(1)) / f64::from(longest)).clamp(1e-4, 1.0) as f32;
    let (rgba, width, height) = crate::render::linear_still(&Arc::new(alone), comp, frame, scale)
        .ok_or(BridgeError::NothingToRead)?;
    let comp_width = doc.comp(comp).map_or(1, |c| c.width.max(1));
    Ok(NodeInput {
        width,
        height,
        rgba,
        scale: width as f32 / comp_width as f32,
    })
}

/// `effects` cut as `cut` says. An effect the stack doesn't hold is refused.
fn cut_stack(effects: &mut Vec<EffectInstance>, cut: StackCut) -> Result<(), BridgeError> {
    match cut {
        StackCut::Whole => {}
        StackCut::Before(id) => {
            let mut at = effects
                .iter()
                .position(|e| e.id == id)
                .ok_or(BridgeError::InvalidEffect)?;
            let joins = |e: &EffectInstance| {
                e.effect.match_name == "correction"
                    && matches!(e.param("combine"), Some(EffectValue::Choice(c)) if *c != 0)
            };
            while at > 0 && effects.get(at).is_some_and(joins) {
                at -= 1;
            }
            effects.truncate(at);
        }
    }
    Ok(())
}

/// The node's rows and the picture coming into it.
#[frb(ignore)]
struct Solving {
    node: Correction,
    input: NodeInput,
}

impl CompositionReference {
    /// Temperature, Tint, Lift and Gain that take the cast out of shot
    /// `layer`/`clip` at `frame` and set its black and white points, read off
    /// the picture coming into Correction `node`.
    pub fn auto_balance(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
    ) -> Result<Vec<BridgeSolvedRow>, BridgeError> {
        let s = self.solving(layer, clip, frame, node)?;
        rows_out(grade_solve::balance::auto_balance(&s.input.rgba, &s.node))
    }

    /// Lift, Gamma, Gain and Saturation that make the shot at `frame`, coming
    /// into `node`, match shot `ref_layer`/`ref_clip` at `ref_frame` as it
    /// looks, its whole grade on.
    #[allow(clippy::too_many_arguments)]
    pub fn match_shot(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        ref_layer: Uuid,
        ref_clip: Option<Uuid>,
        ref_frame: i64,
    ) -> Result<Vec<BridgeSolvedRow>, BridgeError> {
        let s = self.solving(layer, clip, frame, node)?;
        let reference = node_input_picture(
            &self.snapshot()?,
            self.id,
            ref_layer,
            ref_clip,
            ref_frame,
            StackCut::Whole,
            LONG_EDGE,
        )?;
        rows_out(matching::shot_match(
            &s.input.rgba,
            &reference.rgba,
            &s.node,
        ))
    }

    /// As [`Self::match_shot`], against a still: the `.lumstill` at `still`,
    /// whose picture is display sRGB.
    pub fn match_still(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        still: String,
    ) -> Result<Vec<BridgeSolvedRow>, BridgeError> {
        let png = crate::api::stills::still_png(&still)?;
        let reference = matching::linear_from_srgb8(&png);
        let s = self.solving(layer, clip, frame, node)?;
        rows_out(matching::shot_match(&s.input.rgba, &reference, &s.node))
    }

    /// The Space, mixer and Exposure rows that take a ColorChecker Classic
    /// in the picture coming into `node` to its published colours.
    /// `corners` are the chart's top left, top right, bottom right and
    /// bottom left as eight numbers, x then y, comp pixels, dark skin at the
    /// top left.
    pub fn match_chart(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        corners: Vec<f64>,
    ) -> Result<BridgeChartMatch, BridgeError> {
        if corners.len() != 8 || !corners.iter().all(|c| c.is_finite()) {
            return Err(BridgeError::InvalidParam);
        }
        let s = self.solving(layer, clip, frame, node)?;
        let k = f64::from(s.input.scale);
        let at = |i: usize| [(corners[2 * i] * k) as f32, (corners[2 * i + 1] * k) as f32];
        let samples = chart::chart_samples(
            &s.input.rgba,
            s.input.width,
            s.input.height,
            [at(0), at(1), at(2), at(3)],
        );
        let solved =
            chart::chart_match(&samples, &s.node, &self.primaries()?).map_err(solve_error)?;
        Ok(BridgeChartMatch {
            rows: bridge_rows(&solved.rows),
            before: solved.before.to_vec(),
            after: solved.after.to_vec(),
        })
    }

    /// The qualifier rows that key what lies under the circle at (`x`, `y`)
    /// with `radius`, comp pixels, on the picture coming into `node`, by the
    /// node's qualifier mode (Off reads as HSL and turns it on; 3D adds the
    /// circle's average as one sample).
    #[allow(clippy::too_many_arguments)]
    pub fn pick_qualifier(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        x: f64,
        y: f64,
        radius: f64,
        mode: BridgePickMode,
    ) -> Result<Vec<BridgeSolvedRow>, BridgeError> {
        let s = self.solving(layer, clip, frame, node)?;
        let picked = circle(&s.input, x, y, radius);
        rows_out(pick::qualifier_pick(&picked, pick_mode(mode), &s.node))
    }

    /// The 3D qualifier rows after a stroke through `points` (x then y, comp
    /// pixels) with `radius`, read off the picture coming into `node`: Set
    /// keeps only what the stroke crosses, Add keeps it too, Subtract drops
    /// it. A node not yet in 3D is switched to it.
    #[allow(clippy::too_many_arguments)]
    pub fn pick_qualifier_3d(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        points: Vec<f64>,
        radius: f64,
        mode: BridgePickMode,
    ) -> Result<Vec<BridgeSolvedRow>, BridgeError> {
        if points.len() < 2 || !points.len().is_multiple_of(2) {
            return Err(BridgeError::InvalidParam);
        }
        let s = self.solving(layer, clip, frame, node)?;
        let k = f64::from(s.input.scale);
        let stroke: Vec<[f32; 2]> = points
            .chunks_exact(2)
            .map(|p| [(p[0] * k) as f32, (p[1] * k) as f32])
            .collect();
        let picked = pick3d::stroke_samples(
            &s.input.rgba,
            s.input.width,
            s.input.height,
            &stroke,
            (radius * k) as f32,
        );
        rows_out(pick3d::qualifier_pick_3d(&picked, pick_mode(mode), &s.node))
    }

    /// Where the circle at (`x`, `y`) with `radius`, comp pixels, sits on
    /// hue curve `curve` (its row id) of `node`, for the curve editor to put
    /// three points on.
    #[allow(clippy::too_many_arguments)]
    pub fn pick_hue_curve(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        x: f64,
        y: f64,
        radius: f64,
        curve: String,
    ) -> Result<BridgeCurvePick, BridgeError> {
        let axis = pick::curve_axis(&curve).ok_or(BridgeError::InvalidParam)?;
        let s = self.solving(layer, clip, frame, node)?;
        let picked = circle(&s.input, x, y, radius);
        let got = pick::curve_pick(&picked, axis, &s.node).map_err(solve_error)?;
        Ok(BridgeCurvePick {
            centre: f64::from(got.centre),
            spread: f64::from(got.spread),
        })
    }

    /// The project's working primaries against linear Rec. 709: an OCIO
    /// config's `scene_linear` when the project works in it and the config
    /// can say what that is, Rec. 709 otherwise, as the render takes it.
    #[frb(ignore)]
    fn primaries(&self) -> Result<chart::Primaries, BridgeError> {
        let proj = self.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let loaded = crate::api::colour::with_colour(&state, |c| c.loaded().cloned())?;
        let Some(into) = loaded.as_ref().and_then(|l| l.rec709_to_working()) else {
            return Ok(chart::Primaries::REC709);
        };
        let Ok(back) = lumit_colour::matrix::invert(into) else {
            return Ok(chart::Primaries::REC709);
        };
        let three = |m: &lumit_colour::matrix::Matrix34| {
            [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]]
        };
        Ok(chart::Primaries {
            from_rec709: three(into),
            to_rec709: three(&back),
        })
    }

    /// Effect `node` of the shot, its rows read at comp `frame` and handed
    /// to `read` with its match name. `read` answering `None` refuses it.
    #[frb(ignore)]
    pub(crate) fn read_node<T>(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        read: impl FnOnce(&str, lumit_core::fx::params::Params<'_>) -> Option<T>,
    ) -> Result<T, BridgeError> {
        let doc = self.snapshot()?;
        let c = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let (shown, clip_shown) = shot_in(c, layer, clip)?;
        let frame = frame_in_shot(c, shown, clip_shown, frame)?;
        let stack = clip_shown.map_or(&shown.effects, |c| &c.effects);
        let comp_time = c
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        // A node the shot reaches through a group's or a shared node's
        // reference keeps its keys on the comp's clock.
        let (inst, lt) = match stack.iter().find(|e| e.id == node) {
            Some(inst) => (inst, shot_time(c, shown, clip_shown, frame)?),
            None => {
                let (_, group, part) =
                    groups::holder(&doc, stack, node).ok_or(BridgeError::InvalidEffect)?;
                let inst = groups::group(&doc, group)
                    .and_then(|g| g.part(part).iter().find(|e| e.id == node))
                    .ok_or(BridgeError::InvalidEffect)?;
                (inst, comp_time.0.to_f64())
            }
        };
        let context = Arc::new(lumit_core::expression::ExpressionContext {
            document: doc.clone(),
            comp: Some(self.id),
            layer: Some(layer),
            comp_time: comp_time.0.to_f64(),
            current_depth: 0,
            inputs: None,
        });
        let diag = f64::from(c.width).hypot(f64::from(c.height)) as f32;
        let ops = lumit_core::fx::resolved::resolve_stack(
            std::slice::from_ref(inst),
            lt,
            diag,
            1.0,
            &MarkerContext::NONE,
            context,
        );
        let read = ops
            .iter()
            .next()
            .and_then(|fx| read(fx.def.schema().match_name, fx.params));
        read.ok_or(BridgeError::InvalidEffect)
    }

    /// Correction `node` of the shot, its rows read at `frame`, and the
    /// picture coming into it.
    #[frb(ignore)]
    fn solving(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
    ) -> Result<Solving, BridgeError> {
        let schema = &<Correction as EffectMetadata>::SCHEMA;
        let node_rows = self.read_node(layer, clip, frame, node, |name, p| {
            (name == schema.match_name).then(|| Correction::read(p))
        })?;
        let input = node_input_picture(
            &self.snapshot()?,
            self.id,
            layer,
            clip,
            frame,
            StackCut::Before(node),
            LONG_EDGE,
        )?;
        Ok(Solving {
            node: node_rows,
            input,
        })
    }
}

/// The colours under a circle given in comp pixels.
fn circle(input: &NodeInput, x: f64, y: f64, radius: f64) -> Vec<[f32; 3]> {
    let k = input.scale;
    pick::circle_samples(
        &input.rgba,
        input.width,
        input.height,
        [x as f32 * k, y as f32 * k],
        radius as f32 * k,
    )
}

fn pick_mode(mode: BridgePickMode) -> pick::PickMode {
    match mode {
        BridgePickMode::Set => pick::PickMode::Set,
        BridgePickMode::Add => pick::PickMode::Add,
        BridgePickMode::Subtract => pick::PickMode::Subtract,
    }
}

fn solve_error(e: SolveError) -> BridgeError {
    match e {
        SolveError::Empty => BridgeError::NothingToRead,
    }
}

fn rows_out(rows: Result<Rows, SolveError>) -> Result<Vec<BridgeSolvedRow>, BridgeError> {
    rows.map(|r| bridge_rows(&r)).map_err(solve_error)
}

/// The rows as the panel writes them: static values, alpha at 1.
fn bridge_rows(rows: &Rows) -> Vec<BridgeSolvedRow> {
    let scalar = |v: f32| BridgeScalar::Static(f64::from(v));
    rows.iter()
        .map(|(id, v)| BridgeSolvedRow {
            id: (*id).to_owned(),
            value: match *v {
                RowValue::Float(f) => BridgeEffectValue::Float(scalar(f)),
                RowValue::Colour([r, g, b]) => BridgeEffectValue::Colour(BridgeColour {
                    r: scalar(r),
                    g: scalar(g),
                    b: scalar(b),
                    a: scalar(1.0),
                }),
                RowValue::Choice(c) => BridgeEffectValue::Choice(c),
                RowValue::Bool(b) => BridgeEffectValue::Bool(b),
                RowValue::Shape(s) => {
                    BridgeEffectValue::Shape(s.points().iter().map(|p| p.to_vec()).collect())
                }
            },
        })
        .collect()
}
