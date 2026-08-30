use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Keyframe, Property};
use lumit_core::model::{EffectInstance, Layer};
use lumit_core::time::{CompTime, Duration, Rational, SourceTime};

use uuid::Uuid;

use crate::api::{
    composition::{bridge_marker, core_markers, BridgeMarker},
    effect::{
        BridgeEffectInstance, BridgeKeyframe, BridgeRational, BridgeScalar, BridgeSideInterp,
    },
    footage::FootageReference,
    project_item::ItemReference,
    state::{LumitBridgeState, PROJECTS},
    BridgeError,
};

/// A layer's on/off switches, read as a group because the Timeline draws them
/// as one column block and reading them one at a time would be six crossings
/// per row per frame.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeLayerSwitches {
    pub visible: bool,
    pub audible: bool,
    pub locked: bool,
    pub solo: bool,
    /// 2.5D: positions in z and honours the active camera (K-023).
    pub three_d: bool,
    /// The fx switch: bypass the whole effect stack (docs/08 §1.5).
    pub fx: bool,
    /// Per-layer motion blur (K-120); only blurs when the comp's master
    /// shutter is also on.
    pub motion_blur: bool,
    /// Precomp layers only: collapse transformations (docs/06 §1.4).
    pub collapse: bool,
    /// Shy (docs/07 §4.2): hidden from the Timeline's list while the comp's
    /// shy filter is on. Never changes what renders.
    pub shy: bool,
    /// Guide (K-497): drawn in the Viewer and absent from every delivered
    /// file, at every depth — a reference the file does not carry. The one
    /// switch here that changes what an export writes without changing what
    /// the Viewer shows.
    #[frb(default = false)]
    pub guide: bool,
    /// Accepts lights (K-361): whether the comp's Light layers shade this one.
    /// Defaults on, and does nothing at all in a comp with no lights.
    pub accepts_lights: bool,
    /// The adjustment switch (K-537): the layer's own picture is set aside and
    /// its effect stack runs on the composite beneath it.
    ///
    /// True for a layer born an adjustment as well as one switched into being
    /// one — the frontend draws the cell from this and never from the kind.
    #[frb(default = false)]
    pub adjustment: bool,
}

/// One vertex of a mask's path (K-222): where it sits in **layer space**, and
/// the two tangent handles that shape the curve either side of it.
///
/// Tangents are offsets *from* the vertex, in the same layer pixels — the shape
/// `lumit-core`'s `mask::Vertex` uses, carried across unchanged so a path never
/// changes meaning by crossing the bridge. A corner vertex is one with both
/// tangents at zero.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeVertex {
    pub x: f64,
    pub y: f64,
    pub tan_in_x: f64,
    pub tan_in_y: f64,
    pub tan_out_x: f64,
    pub tan_out_y: f64,
}

impl BridgeVertex {
    /// One engine vertex, read across the seam.
    #[frb(ignore)]
    pub(crate) fn read(v: &lumit_core::mask::Vertex) -> Self {
        Self {
            x: v.pos.0,
            y: v.pos.1,
            tan_in_x: v.tan_in.0,
            tan_in_y: v.tan_in.1,
            tan_out_x: v.tan_out.0,
            tan_out_y: v.tan_out.1,
        }
    }

    /// The engine vertex this describes.
    #[frb(ignore)]
    pub(crate) fn write(&self) -> lumit_core::mask::Vertex {
        lumit_core::mask::Vertex {
            pos: (self.x, self.y),
            tan_in: (self.tan_in_x, self.tan_in_y),
            tan_out: (self.tan_out_x, self.tan_out_y),
        }
    }
}

/// One piece of vector art on a shape layer (K-237): a path, and how it is
/// painted.
///
/// The path is `BridgeVertex`, the same vertices a mask crosses with: one path
/// type in the document, drawn by two things.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShapeItem {
    pub id: Uuid,
    pub name: String,
    pub vertices: Vec<BridgeVertex>,
    pub closed: bool,
    /// The colour inside the path. `None` draws no fill.
    pub fill: Option<crate::api::assets::BridgeColourRgba>,
    /// The outline's colour, and its width in layer pixels. `None` draws no
    /// outline; a width of zero draws none either.
    pub stroke: Option<crate::api::assets::BridgeColourRgba>,
    pub stroke_width: f64,
    /// 0..100.
    pub opacity: f64,
    /// **Trim paths** (K-551): where along the path's own length the art begins
    /// and ends, as a per cent, and how far the pair is slid along it in
    /// degrees. Animatable on the layer's own clock (K-213), exactly as a
    /// stroke's write-on is, so the Timeline rows carry the same stopwatch and
    /// the same diamonds.
    pub trim_start: BridgeScalar,
    pub trim_end: BridgeScalar,
    pub trim_offset: BridgeScalar,
    /// **Dashes** (K-552): the outline's dash and gap lengths in layer pixels,
    /// alternating — dash, gap, dash, gap. Empty is a solid outline.
    /// `dash_offset` is how far along the path the pattern starts, in the same
    /// pixels.
    pub dashes: Vec<BridgeScalar>,
    pub dash_offset: BridgeScalar,
    /// **A gradient fill** (K-555): 0 is the flat [`fill`](Self::fill), 1 ramps
    /// from it linearly to `gradient_colour` and 2 ramps radially, the end
    /// point sitting on the outer edge. The two points are in layer pixels —
    /// the art's own coordinates — and animate; what a ramp *is* does not.
    pub gradient: u32,
    pub gradient_colour: Option<crate::api::assets::BridgeColourRgba>,
    pub gradient_start_x: BridgeScalar,
    pub gradient_start_y: BridgeScalar,
    pub gradient_end_x: BridgeScalar,
    pub gradient_end_y: BridgeScalar,
    /// **A boolean combine** (K-605): how this item joins the item **before**
    /// it in the list — 0 draws it on its own, 1 unions, 2 subtracts this one
    /// from that one, 3 keeps only what both cover, 4 keeps only what one of
    /// them covers. The run that makes is drawn once, with the paint and the
    /// modifiers of the item that starts it.
    pub combine: u32,
    /// This item's **shape** keys — empty when its path does not animate
    /// (K-606). Composition time, carried out by the layer's start offset
    /// exactly as a mask's path keys cross (K-224).
    ///
    /// The shapes themselves do not cross: a key holds a whole path, which the
    /// frontend edits through the drawing tools rather than by sending a list
    /// of them. `value` counts the keys up, so the graph draws the *rate* the
    /// shape is changing at, which is the one curve a path can honestly draw
    /// (K-344).
    pub path_keys: Vec<BridgeKeyframe>,
    /// **Offset paths** (K-554): how far the outline is pushed out of the path,
    /// in layer pixels; negative pulls it in and zero is the path itself.
    pub offset_amount: BridgeScalar,
    /// **The repeater** (K-553): how many copies of the item are drawn, which
    /// copy the original is (`repeat_offset`), and the transform each copy is
    /// one more step of — moved by `repeat_position_*` layer pixels, turned by
    /// `repeat_rotation` degrees and scaled by `repeat_scale` per cent, all
    /// about `repeat_anchor_*`. The copies fade evenly from
    /// `repeat_start_opacity` to `repeat_end_opacity`.
    ///
    /// A still count of one is no repeater at all, which is what every shape is
    /// until somebody asks for more.
    pub repeat_copies: BridgeScalar,
    pub repeat_offset: BridgeScalar,
    pub repeat_anchor_x: BridgeScalar,
    pub repeat_anchor_y: BridgeScalar,
    pub repeat_position_x: BridgeScalar,
    pub repeat_position_y: BridgeScalar,
    pub repeat_rotation: BridgeScalar,
    pub repeat_scale: BridgeScalar,
    pub repeat_start_opacity: BridgeScalar,
    pub repeat_end_opacity: BridgeScalar,
}

impl BridgeShapeItem {
    #[frb(ignore)]
    fn read_at(item: &lumit_core::shape::ShapeItem, offset: Rational) -> Self {
        Self {
            id: item.id,
            name: item.name.clone(),
            vertices: item.path.vertices.iter().map(BridgeVertex::read).collect(),
            closed: item.path.closed,
            fill: item.fill.map(crate::api::assets::colour_of),
            stroke: item.stroke.map(crate::api::assets::colour_of),
            stroke_width: item.stroke_width,
            opacity: item.opacity,
            trim_start: BridgeScalar::read_at(&item.trim_start, offset),
            trim_end: BridgeScalar::read_at(&item.trim_end, offset),
            trim_offset: BridgeScalar::read_at(&item.trim_offset, offset),
            dashes: item
                .dashes
                .iter()
                .map(|d| BridgeScalar::read_at(d, offset))
                .collect(),
            dash_offset: BridgeScalar::read_at(&item.dash_offset, offset),
            gradient: item.gradient,
            gradient_colour: item.gradient_colour.map(crate::api::assets::colour_of),
            gradient_start_x: BridgeScalar::read_at(&item.gradient_start_x, offset),
            gradient_start_y: BridgeScalar::read_at(&item.gradient_start_y, offset),
            gradient_end_x: BridgeScalar::read_at(&item.gradient_end_x, offset),
            gradient_end_y: BridgeScalar::read_at(&item.gradient_end_y, offset),
            combine: item.combine,
            path_keys: item
                .path_keys
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    let time = k.time.checked_add(offset).unwrap_or(k.time);
                    BridgeKeyframe {
                        time: BridgeRational {
                            num: time.num(),
                            den: time.den(),
                        },
                        value: i as f64,
                        interp_in: BridgeSideInterp::read(k.interp_in),
                        interp_out: BridgeSideInterp::read(k.interp_out),
                    }
                })
                .collect(),
            offset_amount: BridgeScalar::read_at(&item.offset_amount, offset),
            repeat_copies: BridgeScalar::read_at(&item.repeat_copies, offset),
            repeat_offset: BridgeScalar::read_at(&item.repeat_offset, offset),
            repeat_anchor_x: BridgeScalar::read_at(&item.repeat_anchor_x, offset),
            repeat_anchor_y: BridgeScalar::read_at(&item.repeat_anchor_y, offset),
            repeat_position_x: BridgeScalar::read_at(&item.repeat_position_x, offset),
            repeat_position_y: BridgeScalar::read_at(&item.repeat_position_y, offset),
            repeat_rotation: BridgeScalar::read_at(&item.repeat_rotation, offset),
            repeat_scale: BridgeScalar::read_at(&item.repeat_scale, offset),
            repeat_start_opacity: BridgeScalar::read_at(&item.repeat_start_opacity, offset),
            repeat_end_opacity: BridgeScalar::read_at(&item.repeat_end_opacity, offset),
        }
    }

    /// The engine's item this describes. Public to the crate because the
    /// composition builds a whole layer out of a list of them.
    #[frb(ignore)]
    pub(crate) fn write_item(
        &self,
        offset: Rational,
    ) -> Result<lumit_core::shape::ShapeItem, BridgeError> {
        Ok(lumit_core::shape::ShapeItem {
            id: self.id,
            name: self.name.clone(),
            path: lumit_core::mask::BezierPath {
                vertices: self.vertices.iter().map(BridgeVertex::write).collect(),
                closed: self.closed,
            },
            fill: self.fill.map(crate::api::assets::linear_of),
            stroke: self.stroke.map(crate::api::assets::linear_of),
            stroke_width: self.stroke_width.clamp(0.0, 10_000.0),
            opacity: self.opacity.clamp(0.0, 100.0),
            // Per cent of the path's own length, so anything outside 0..100 is
            // a number that could only ever draw wrongly — clamped here, every
            // key of it, exactly as a stroke's trim is (K-549). The offset is
            // degrees and wraps, so it is left alone.
            trim_start: clamped_property(&self.trim_start, offset, 0.0, 100.0)?,
            trim_end: clamped_property(&self.trim_end, offset, 0.0, 100.0)?,
            trim_offset: clamped_property(&self.trim_offset, offset, -360_000.0, 360_000.0)?,
            // Lengths in layer pixels: a negative dash is not a shorter dash,
            // it is a number with no meaning, so zero is where it lands. The
            // offset may be negative — it slides both ways.
            dashes: self
                .dashes
                .iter()
                .map(|d| clamped_property(d, offset, 0.0, 100_000.0))
                .collect::<Result<_, _>>()?,
            dash_offset: clamped_property(&self.dash_offset, offset, -100_000.0, 100_000.0)?,
            // Two readings and no third: a number naming neither is the flat
            // fill, which is the answer that draws something.
            gradient: if self.gradient <= 2 { self.gradient } else { 0 },
            gradient_colour: self.gradient_colour.map(crate::api::assets::linear_of),
            // The art's own coordinates, held to the same reach a vertex has.
            gradient_start_x: clamped_property(
                &self.gradient_start_x,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            gradient_start_y: clamped_property(
                &self.gradient_start_y,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            gradient_end_x: clamped_property(&self.gradient_end_x, offset, -100_000.0, 100_000.0)?,
            gradient_end_y: clamped_property(&self.gradient_end_y, offset, -100_000.0, 100_000.0)?,
            // Five readings and no sixth: a number naming none of them draws
            // the art on its own, which is the answer that shows something.
            combine: if self.combine <= 4 { self.combine } else { 0 },
            // What this type does not carry. An item edited from the frontend
            // must not LOSE its shape keys, so `write_item_over` patches them
            // back from the item it is replacing; this bare form is only for
            // art that did not exist a moment ago, which has none.
            path_keys: Vec::new(),
            // Layer pixels, out or in: both directions mean something, so only
            // the far ends are held.
            offset_amount: clamped_property(&self.offset_amount, offset, -100_000.0, 100_000.0)?,
            // The count is what the engine holds to 1..MAX_COPIES as it draws,
            // so the clamp here only keeps a wild number out of the document;
            // the offset is a copy index and may be negative, which is what
            // puts copies behind the original.
            repeat_copies: clamped_property(
                &self.repeat_copies,
                offset,
                1.0,
                lumit_core::shape::MAX_COPIES as f64,
            )?,
            repeat_offset: clamped_property(
                &self.repeat_offset,
                offset,
                -(lumit_core::shape::MAX_COPIES as f64),
                lumit_core::shape::MAX_COPIES as f64,
            )?,
            repeat_anchor_x: clamped_property(
                &self.repeat_anchor_x,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            repeat_anchor_y: clamped_property(
                &self.repeat_anchor_y,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            repeat_position_x: clamped_property(
                &self.repeat_position_x,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            repeat_position_y: clamped_property(
                &self.repeat_position_y,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            // Degrees, which wrap; a scale of zero collapses a copy to nothing
            // and a negative one mirrors it, both of which are drawings.
            repeat_rotation: clamped_property(
                &self.repeat_rotation,
                offset,
                -360_000.0,
                360_000.0,
            )?,
            repeat_scale: clamped_property(&self.repeat_scale, offset, -10_000.0, 10_000.0)?,
            repeat_start_opacity: clamped_property(&self.repeat_start_opacity, offset, 0.0, 100.0)?,
            repeat_end_opacity: clamped_property(&self.repeat_end_opacity, offset, 0.0, 100.0)?,
            extra: serde_json::Map::new(),
        })
    }

    /// [`Self::write_item`], but keeping what `previous` carries and this type
    /// does not describe: the **shape keyframes** (K-606) and the
    /// forward-compatibility `extra` a newer Lumit may have written (docs/10
    /// §1.1 makes preserving it mandatory).
    ///
    /// **A shape edit on a morphing item lands on the key under the
    /// playhead**, exactly as a mask's does (K-340). Once a path is keyed,
    /// `path` is no longer what the item draws — `path_at` reads the keys — so
    /// writing the dragged vertices there would move nothing at all and the art
    /// would appear frozen under the pointer. `at` is where the playhead is;
    /// without it — an edit that is not a shape edit, such as an opacity drag —
    /// the keys are carried through untouched.
    #[frb(ignore)]
    pub(crate) fn write_item_over(
        &self,
        previous: &lumit_core::shape::ShapeItem,
        offset: Rational,
        at: Option<Rational>,
    ) -> Result<lumit_core::shape::ShapeItem, BridgeError> {
        let mut written = lumit_core::shape::ShapeItem {
            path_keys: previous.path_keys.clone(),
            extra: previous.extra.clone(),
            ..self.write_item(offset)?
        };
        if let (false, Some(at)) = (written.path_keys.is_empty(), at) {
            let at = at
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)?;
            let path = std::mem::replace(&mut written.path, previous.path.clone());
            match written.path_keys.iter_mut().find(|k| k.time == at) {
                Some(key) => key.path = path,
                None => {
                    let i = written
                        .path_keys
                        .iter()
                        .position(|k| k.time > at)
                        .unwrap_or(written.path_keys.len());
                    written.path_keys.insert(
                        i,
                        lumit_core::mask::PathKeyframe {
                            time: at,
                            path,
                            interp_in: lumit_core::anim::SideInterp::Linear,
                            interp_out: lumit_core::anim::SideInterp::Linear,
                        },
                    );
                }
            }
        }
        Ok(written)
    }
}

/// What a paint stroke does to the pixels under it (K-227).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePaintMode {
    /// Lay the stroke's colour down.
    Paint,
    /// Take alpha away.
    Erase,
    /// Copy from elsewhere on the same layer, by the stroke's clone offset.
    Clone,
}

/// The shape one dab of the brush leaves (K-548). Not a brush-tip system:
/// two shapes, both measured from the dab's centre out to half the stroke's
/// width and both softened by the same hardness ramp.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeBrushShape {
    /// A circle — the brush every stroke had before there was a choice.
    Round,
    /// A square with flat sides and square corners.
    Square,
}

/// One point of a stroke's path, in layer pixels.
///
/// Named for the stroke rather than called `BridgePoint`, because that name is
/// already an *animatable* point parameter on an effect — two quite different
/// things, and the bridge's type names are flat across the whole seam.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeStrokePoint {
    pub x: f64,
    pub y: f64,
    /// How hard the stylus was pressed here, 0..1 (K-583). **1.0 is "no stylus
    /// said otherwise"** — a mouse, a finger, a tablet with the brush's
    /// pressure toggle off — and a stroke whose points are all 1.0 is stored
    /// with no pressures at all, so it is the stroke it would have been before
    /// any of this existed.
    pub pressure: f64,
}

/// One paint stroke on a layer (K-227): the path the pointer took, and how it
/// was painted.
///
/// A **polyline**, not a bezier — a stroke is a record of a gesture rather than
/// a shape to be edited vertex by vertex, which is the difference between this
/// and [`BridgeMask`]. Layer space, like everything else that travels with a
/// layer's transform.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeStroke {
    pub id: Uuid,
    pub name: String,
    pub points: Vec<BridgeStrokePoint>,
    pub colour: crate::api::assets::BridgeColourRgba,
    /// The brush's diameter in layer pixels.
    pub width: f64,
    /// 0 fully soft, 1 a hard edge. The same ramp whatever the shape is.
    pub hardness: f64,
    pub shape: BridgeBrushShape,
    /// 0..100.
    pub opacity: f64,
    /// Where along the path the mark begins and ends, as a per cent of the
    /// stroke's own length (K-549). Animatable exactly as a mask's opacity is,
    /// on the layer's own clock (K-213), so the Timeline row carries the same
    /// stopwatch and the same diamonds. Hold Start at 0 and key End 0 → 100
    /// and the stroke draws itself on.
    pub start: BridgeScalar,
    pub end: BridgeScalar,
    pub mode: BridgePaintMode,
    /// How the mark combines with what is already on the layer (K-550), as an
    /// index into [`list_blend_modes`] — the same list and the same convention
    /// a layer's own blend crosses on. Ignored by an eraser, which takes alpha
    /// away and never touches colour.
    pub blend: u32,
    /// Where a clone's pixels are copied from, as an offset in layer pixels.
    pub clone_offset_x: f64,
    pub clone_offset_y: f64,
}

impl BridgeStroke {
    #[frb(ignore)]
    fn read_at(stroke: &lumit_core::paint::PaintStroke, offset: Rational) -> Self {
        Self {
            id: stroke.id,
            name: stroke.name.clone(),
            points: stroke
                .points
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| BridgeStrokePoint {
                    x,
                    y,
                    // No pressures is the constant 1.0, and so is a list that
                    // stops short — the frontend never has to ask which.
                    pressure: stroke.pressures.get(i).copied().unwrap_or(1.0),
                })
                .collect(),
            colour: crate::api::assets::colour_of(stroke.colour),
            width: stroke.width,
            hardness: stroke.hardness,
            shape: match stroke.shape {
                lumit_core::paint::BrushShape::Round => BridgeBrushShape::Round,
                lumit_core::paint::BrushShape::Square => BridgeBrushShape::Square,
            },
            opacity: stroke.opacity,
            start: BridgeScalar::read_at(&stroke.start, offset),
            end: BridgeScalar::read_at(&stroke.end, offset),
            mode: match stroke.mode {
                lumit_core::paint::PaintMode::Paint => BridgePaintMode::Paint,
                lumit_core::paint::PaintMode::Erase => BridgePaintMode::Erase,
                lumit_core::paint::PaintMode::Clone => BridgePaintMode::Clone,
            },
            blend: lumit_core::model::BlendMode::ALL
                .iter()
                .position(|b| *b == stroke.blend)
                .unwrap_or(0) as u32,
            clone_offset_x: stroke.clone_offset.0,
            clone_offset_y: stroke.clone_offset.1,
        }
    }

    /// The engine's stroke this describes. Every number that would render
    /// wrongly for ever after is clamped here rather than trusted, exactly as
    /// a mask's opacity is.
    #[frb(ignore)]
    pub(crate) fn write_at(
        &self,
        offset: Rational,
    ) -> Result<lumit_core::paint::PaintStroke, BridgeError> {
        Ok(lumit_core::paint::PaintStroke {
            id: self.id,
            name: self.name.clone(),
            points: self.points.iter().map(|p| (p.x, p.y)).collect(),
            // All-1.0 is stored as nothing at all (K-583): a mouse-drawn stroke
            // has to be the stroke it was before there was a stylus, in the
            // file as well as on the screen, or every old project's bytes move
            // the first time it is opened and saved.
            pressures: if self.points.iter().all(|p| p.pressure >= 1.0) {
                Vec::new()
            } else {
                self.points
                    .iter()
                    .map(|p| p.pressure.clamp(0.0, 1.0))
                    .collect()
            },
            colour: crate::api::assets::linear_of(self.colour),
            width: self.width.clamp(0.0, 10_000.0),
            hardness: self.hardness.clamp(0.0, 1.0),
            shape: match self.shape {
                BridgeBrushShape::Round => lumit_core::paint::BrushShape::Round,
                BridgeBrushShape::Square => lumit_core::paint::BrushShape::Square,
            },
            opacity: self.opacity.clamp(0.0, 100.0),
            // Per cent of the stroke's own length, so anything outside 0..100
            // is a number that could only ever render wrongly — clamped here,
            // every key of it, exactly as a mask's opacity is (K-549).
            start: clamped_property(&self.start, offset, 0.0, 100.0)?,
            end: clamped_property(&self.end, offset, 0.0, 100.0)?,
            mode: match self.mode {
                BridgePaintMode::Paint => lumit_core::paint::PaintMode::Paint,
                BridgePaintMode::Erase => lumit_core::paint::PaintMode::Erase,
                BridgePaintMode::Clone => lumit_core::paint::PaintMode::Clone,
            },
            // An index past the end of the list is Normal rather than an
            // error: it is a frontend that has fallen behind, and a stroke
            // that lays its colour down is the honest reading of one.
            blend: lumit_core::model::BlendMode::ALL
                .get(self.blend as usize)
                .copied()
                .unwrap_or_default(),
            clone_offset: (self.clone_offset_x, self.clone_offset_y),
            extra: serde_json::Map::new(),
        })
    }
}

/// One mask on a layer: a bezier path that gates the layer's alpha before its
/// effects and transform (docs/06 render order).
///
/// The path is in **layer space** — the same coordinates the layer's own pixels
/// use — so a mask travels with the layer's transform for free, exactly as it
/// does in After Effects.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeMask {
    pub id: Uuid,
    pub name: String,
    pub vertices: Vec<BridgeVertex>,
    /// Whether the path joins its last vertex back to its first. An open path
    /// gates nothing yet; it is a shape being drawn.
    pub closed: bool,
    pub inverted: bool,
    /// 0..100, and animatable exactly as a transform property is (K-340) — so
    /// the Timeline row carries the same stopwatch and the same ◄ ◆ ► as every
    /// other property. Times are the layer's own, as everywhere else (K-213).
    pub opacity: BridgeScalar,
    /// How this mask combines with the ones above it.
    pub mode: BridgeMaskMode,
    /// Width of the soft edge in layer pixels; 0 is the hard antialiased edge.
    pub feather: BridgeScalar,
    /// A width of its own for each **vertex**, in layer pixels (K-545), each
    /// animatable exactly as [`Self::feather`] is. Empty — the ordinary mask —
    /// means one width all the way round, and is what the Timeline shows no
    /// per-point rows for.
    ///
    /// Positional: entry *i* belongs to vertex *i* of [`Self::vertices`], so a
    /// caller changing the shape and the widths in one write must send the two
    /// lists agreeing with each other.
    pub vertex_feather: Vec<BridgeScalar>,
    /// Grow (+) or shrink (−) the shape, in layer pixels.
    pub expansion: BridgeScalar,
    /// This mask's **shape** keys — empty when the path does not animate.
    /// Composition time, carried out by the layer's start offset exactly as a
    /// scalar's keyframe times cross (K-213).
    ///
    /// The shapes themselves do not cross: a key holds a whole path, which the
    /// frontend edits through the drawing tools rather than by sending a list
    /// of them (K-339). What crosses is where the keys are and how they ease —
    /// which is everything the lane and the graph need.
    ///
    /// **`value` is the interpolation parameter, counted up** (K-344): key *i*
    /// carries *i*, so every span rises by exactly 1 as the shape crosses from
    /// one key to the next. The number itself means nothing to look at, but its
    /// *slope* is the rate the shape is changing at — which is the one curve a
    /// path can honestly draw, and the one After Effects draws for a mask path.
    pub path_keys: Vec<BridgeKeyframe>,
}

/// A mask's scalar as an engine [`Property`], with every value it can take held
/// inside `[lo, hi]`.
///
/// **Clamping an animation means clamping its keys**, not just the number the
/// playhead happens to be over: a mask keyed to −40 % opacity three seconds
/// away is just as wrong as one set to −40 % now, and it would arrive the
/// moment the playhead did. An expression cannot be clamped here at all — it is
/// a string until it runs — so it is passed through and the renderer's own
/// reads keep their clamps.
#[frb(ignore)]
fn clamped_property(
    scalar: &BridgeScalar,
    offset: Rational,
    lo: f64,
    hi: f64,
) -> Result<Property, BridgeError> {
    let animation = match scalar.animation_at(offset)? {
        Animation::Static(v) => Animation::Static(v.clamp(lo, hi)),
        Animation::Keyframed(keys) => Animation::Keyframed(
            keys.into_iter()
                .map(|k| Keyframe {
                    value: k.value.clamp(lo, hi),
                    ..k
                })
                .collect(),
        ),
        expression => expression,
    };
    Ok(Property {
        animation,
        extra: serde_json::Map::new(),
    })
}

/// [`lumit_core::mask::MaskMode`] across the bridge. Its own enum because the
/// engine's types do not cross (docs/17 §Types), and named the same so the two
/// cannot drift apart unnoticed.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BridgeMaskMode {
    /// Geometry only: the path is editable and gates nothing.
    None,
    #[default]
    Add,
    Subtract,
    Intersect,
    /// The greater of this mask and the stack below it (K-545).
    Lighten,
    /// The lesser of the two (K-545).
    Darken,
    Difference,
}

impl BridgeMaskMode {
    #[frb(ignore)]
    fn read(mode: lumit_core::mask::MaskMode) -> Self {
        match mode {
            lumit_core::mask::MaskMode::None => Self::None,
            lumit_core::mask::MaskMode::Add => Self::Add,
            lumit_core::mask::MaskMode::Subtract => Self::Subtract,
            lumit_core::mask::MaskMode::Intersect => Self::Intersect,
            lumit_core::mask::MaskMode::Lighten => Self::Lighten,
            lumit_core::mask::MaskMode::Darken => Self::Darken,
            lumit_core::mask::MaskMode::Difference => Self::Difference,
        }
    }

    #[frb(ignore)]
    fn write(self) -> lumit_core::mask::MaskMode {
        match self {
            Self::None => lumit_core::mask::MaskMode::None,
            Self::Add => lumit_core::mask::MaskMode::Add,
            Self::Subtract => lumit_core::mask::MaskMode::Subtract,
            Self::Intersect => lumit_core::mask::MaskMode::Intersect,
            Self::Lighten => lumit_core::mask::MaskMode::Lighten,
            Self::Darken => lumit_core::mask::MaskMode::Darken,
            Self::Difference => lumit_core::mask::MaskMode::Difference,
        }
    }
}

impl BridgeMask {
    #[frb(ignore)]
    fn read_at(mask: &lumit_core::mask::Mask, offset: Rational) -> Self {
        Self {
            id: mask.id,
            name: mask.name.clone(),
            vertices: mask.path.vertices.iter().map(BridgeVertex::read).collect(),
            closed: mask.path.closed,
            inverted: mask.inverted,
            opacity: BridgeScalar::read_at(&mask.opacity, offset),
            mode: BridgeMaskMode::read(mask.mode),
            feather: BridgeScalar::read_at(&mask.feather, offset),
            vertex_feather: mask
                .vertex_feather
                .iter()
                .map(|p| BridgeScalar::read_at(p, offset))
                .collect(),
            expansion: BridgeScalar::read_at(&mask.expansion, offset),
            path_keys: mask
                .path_keys
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    let time = k.time.checked_add(offset).unwrap_or(k.time);
                    BridgeKeyframe {
                        time: BridgeRational {
                            num: time.num(),
                            den: time.den(),
                        },
                        value: i as f64,
                        interp_in: BridgeSideInterp::read(k.interp_in),
                        interp_out: BridgeSideInterp::read(k.interp_out),
                    }
                })
                .collect(),
        }
    }

    /// The engine's mask this describes. `id` is kept, so an edit names the
    /// mask it came from; a caller making a *new* mask sends a fresh uuid.
    #[frb(ignore)]
    pub(crate) fn write(&self, offset: Rational) -> Result<lumit_core::mask::Mask, BridgeError> {
        Ok(lumit_core::mask::Mask {
            id: self.id,
            name: self.name.clone(),
            path: lumit_core::mask::BezierPath {
                vertices: self.vertices.iter().map(BridgeVertex::write).collect(),
                closed: self.closed,
            },
            inverted: self.inverted,
            // A mask with an absurd opacity is a mask that renders wrongly for
            // ever after; clamped here rather than trusted. Clamping a whole
            // animation means clamping every key it holds.
            opacity: clamped_property(&self.opacity, offset, 0.0, 100.0)?,
            // What this type does not carry yet. A mask edited from the frontend
            // must not LOSE these, so `set_mask` patches them back from the mask
            // it is replacing (see `write_over`); this bare form is only for a
            // mask that did not exist a moment ago, which has neither.
            path_keys: Vec::new(),
            extra: serde_json::Map::new(),
            mode: self.mode.write(),
            // Same reasoning as opacity. A negative feather is not a thing, and
            // both are bounded so a typo cannot ask for a distance field the
            // size of a continent. The ceiling is generous: 5000 layer pixels
            // is wider than any comp anyone is masking.
            feather: clamped_property(&self.feather, offset, 0.0, 5000.0)?,
            // Each per-vertex width is bounded exactly as the one width is,
            // and for the same reason (K-545).
            vertex_feather: self
                .vertex_feather
                .iter()
                .map(|s| clamped_property(s, offset, 0.0, 5000.0))
                .collect::<Result<Vec<_>, _>>()?,
            expansion: clamped_property(&self.expansion, offset, -5000.0, 5000.0)?,
        })
    }

    /// [`Self::write`], but keeping what `previous` carries and this type does
    /// not describe: the path keyframes, and the forward-compatibility `extra`
    /// a newer Lumit may have written (docs/10 §1.1 makes preserving it
    /// mandatory).
    ///
    /// **Why this exists.** `BridgeMask` is the only bridge type that rebuilds
    /// its engine value field by field rather than patching the one it read, so
    /// every field the engine grows and the bridge does not is silently dropped
    /// the moment the frontend edits that mask. Dragging a mask's opacity would
    /// otherwise delete its animation.
    ///
    /// **A shape edit on an animated mask lands on the key under the
    /// playhead** (K-340). Once a path is keyed, `path` is no longer what the
    /// mask draws — `path_at` reads the keys — so writing the dragged vertices
    /// there would move nothing at all and the shape would appear frozen under
    /// the pointer. `at` is where the playhead is; with it, the vertices update
    /// the key sitting at that time or plant one holding them, which is what
    /// dragging a keyframed value does everywhere else (docs/07 §4.3). Without
    /// it — an edit that is not a shape edit, such as an opacity drag — the
    /// keys are simply carried through untouched.
    #[frb(ignore)]
    fn write_over(
        &self,
        previous: &lumit_core::mask::Mask,
        offset: Rational,
        at: Option<Rational>,
    ) -> Result<lumit_core::mask::Mask, BridgeError> {
        let mut written = lumit_core::mask::Mask {
            path_keys: previous.path_keys.clone(),
            extra: previous.extra.clone(),
            ..self.write(offset)?
        };
        if let (false, Some(at)) = (written.path_keys.is_empty(), at) {
            let at = at
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)?;
            let path = std::mem::replace(&mut written.path, previous.path.clone());
            match written.path_keys.iter_mut().find(|k| k.time == at) {
                Some(key) => key.path = path,
                None => {
                    let i = written
                        .path_keys
                        .iter()
                        .position(|k| k.time > at)
                        .unwrap_or(written.path_keys.len());
                    written.path_keys.insert(
                        i,
                        lumit_core::mask::PathKeyframe {
                            time: at,
                            path,
                            interp_in: lumit_core::anim::SideInterp::Linear,
                            interp_out: lumit_core::anim::SideInterp::Linear,
                        },
                    );
                }
            }
        }
        Ok(written)
    }
}

/// Which switch an edit names. One enum rather than eight methods so the
/// Timeline's switch column is one handler, and so a new switch cannot be added
/// engine-side without the compiler pointing at every arm here.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLayerSwitch {
    Visible,
    Audible,
    Locked,
    Solo,
    ThreeD,
    Fx,
    MotionBlur,
    Collapse,
    Shy,
    /// K-361: whether the comp's Light layers shade this one.
    AcceptsLights,
    /// K-497: reference-only — the Viewer draws it, no delivered file carries
    /// it. A locked layer refuses this one, unlike shy: it changes what the
    /// export writes.
    Guide,
    /// K-537: the layer sets its own picture aside and grades the composite
    /// beneath it. Refused ([`BridgeError::NotConvertible`]) on the four kinds
    /// with no picture to set aside — Camera, Light, Null, Audio.
    Adjustment,
}

/// Where a layer sits on the comp timeline, in exact rational seconds.
///
/// `start_offset` is where the layer's own time 0 falls, which is what a slip
/// edit moves and what makes trimming the in point *not* re-time the content.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeSpan {
    pub in_point: BridgeRational,
    /// Exclusive; must be after `in_point`.
    pub out_point: BridgeRational,
    pub start_offset: BridgeRational,
}

/// What kind of source a layer has — what the Timeline draws its bar and its
/// label colour from. The payloads the model carries (the footage item, the
/// text document, the clip list) are reached through their own readers rather
/// than duplicated here.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLayerKind {
    Footage,
    Solid,
    Precomp,
    Text,
    /// Vector art as the layer's own picture (K-237).
    Shape,
    Camera,
    Sequence,
    Adjustment,
    /// A Null layer. Named `NullLayer` rather than `Null` only because the
    /// generated Dart enum would otherwise carry a member called `null`, which
    /// is a Dart reserved word (K-206); `lumit-core` keeps `LayerKind::Null`.
    NullLayer,
    /// A Light layer (K-360): a source of light other layers see. Draws no
    /// pixels of its own, like a Camera.
    Light,
    /// An Audio layer (K-435): a footage source contributing sound only. Not a
    /// [`lumit_core::model::LayerKind`] of its own — it is a Footage layer with
    /// `audio_only` set — but the frontend draws it as its own kind (its own
    /// glyph, no thumbnail, no visibility switch), so it is its own kind here.
    Audio,
}

/// How the frontend should draw this layer (K-435): the model's kind, except
/// that a footage layer carrying only sound reads as [`BridgeLayerKind::Audio`].
///
/// One function rather than a `match` at each call site, so the read model and
/// `get_kind` can never disagree about what a layer is.
#[frb(ignore)]
pub(crate) fn bridge_kind(layer: &lumit_core::model::Layer) -> BridgeLayerKind {
    use lumit_core::model::LayerKind as K;
    if layer.audio_only {
        return BridgeLayerKind::Audio;
    }
    match &layer.kind {
        K::Footage { .. } => BridgeLayerKind::Footage,
        K::Solid { .. } => BridgeLayerKind::Solid,
        K::Precomp { .. } => BridgeLayerKind::Precomp,
        K::Text { .. } => BridgeLayerKind::Text,
        K::Camera { .. } => BridgeLayerKind::Camera,
        K::Sequence { .. } => BridgeLayerKind::Sequence,
        K::Adjustment => BridgeLayerKind::Adjustment,
        K::Shape { .. } => BridgeLayerKind::Shape,
        K::Null => BridgeLayerKind::NullLayer,
        K::Light { .. } => BridgeLayerKind::Light,
    }
}

/// A layer's switches as the frontend reads them.
///
/// One function rather than a struct literal at each call site, so the read
/// model and [`LayerReference::get_switches`] can never disagree — the trap
/// [`bridge_kind`] exists to close, on the group beside it.
#[frb(ignore)]
pub(crate) fn bridge_switches(layer: &lumit_core::model::Layer) -> BridgeLayerSwitches {
    let s = layer.switches;
    BridgeLayerSwitches {
        visible: s.visible,
        audible: s.audible,
        locked: s.locked,
        solo: s.solo,
        three_d: s.three_d,
        fx: s.fx,
        motion_blur: s.motion_blur,
        collapse: s.collapse,
        shy: s.shy,
        guide: s.guide,
        accepts_lights: s.accepts_lights,
        // Not in `Switches` — it is a field on the layer (K-537), and this is
        // the one answer for both the flag and the legacy Adjustment kind.
        adjustment: layer.is_adjustment(),
    }
}

/// One clip on a Sequence layer, as the Timeline needs to draw it: where it
/// starts on the layer's own timeline and how long it occupies there.
///
/// The source trim and the retime map are not carried: nothing draws them yet,
/// and a value type that pretends to round-trip what no control can edit is how
/// a write quietly loses information.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeClip {
    pub id: Uuid,
    pub place_start: BridgeRational,
    pub place_duration: BridgeRational,
    /// Where the clip sits on the **comp's** own clock, in frames — so the
    /// expanded row draws with no time-to-frame trip per clip (K-248, K-184).
    ///
    /// A clip's `place_*` are in *layer* time; these carry the layer's own
    /// zero already added. The two are the same number only while that zero
    /// is itself zero, which stopped being true the moment a clip could be
    /// dragged back past the start of its row.
    pub start_frame: i64,
    pub end_frame: i64,
    /// The clip's single playback speed in per cent, or `None` when its map
    /// says something one number cannot — a ramp, or a richer curve. The row
    /// shows the envelope for those rather than a number.
    pub speed_percent: Option<f64>,
    /// Whether the clip carries a map at all. `false` is "plays at source
    /// rate", a different state from a map that happens to be 100%.
    pub retimed: bool,
    /// The map this clip actually plays by, keyed in **clip-local** time —
    /// what the sequence view's envelope draws and edits (K-247, K-248).
    ///
    /// Always present, even for a clip with no map of its own: it then holds
    /// the identity that clip is playing, running from its real trim-in.
    /// [`Self::retimed`] is what says which of the two it is.
    ///
    /// Carried rather than left for the frontend to construct, because
    /// constructing it means knowing where the clip's source starts — and a
    /// frontend that assumed zero sent every clip after a cut back to the top
    /// of its media the moment it was ramped.
    pub retime: crate::api::effect::BridgeScalar,
    /// Where the **whole source** would sit on the comp's clock if none of it
    /// had been trimmed away, in frames — the faint ghost a trimmed clip draws
    /// inside a Sequence lane (K-441, docs/15 §12A.1), the clip-level twin of
    /// the outline a trimmed *layer* already wears.
    ///
    /// `None` when the reach is not knowable, and the engine
    /// ([`lumit_core::sequence::Clip::source_reach`]) decides which cases those
    /// are: a **retimed** clip has none, because its map decides for itself
    /// which source moment each frame shows, and a source whose length could
    /// not be read has none either rather than one pinned to a guess. Nothing
    /// is clamped, so a clip dragged so its source would begin before the row's
    /// origin reports a negative first frame — exactly as a layer's bounds do.
    ///
    /// Carried in comp frames, like `start_frame`/`end_frame` beside it, so the
    /// ghost is one more positioned box and no time↔frame trip rides a rebuild.
    pub reach_start_frame: Option<i64>,
    pub reach_end_frame: Option<i64>,
}

/// How long a clip's source runs, which is the one thing
/// [`lumit_core::sequence::Clip::source_reach`] cannot work out for itself: a
/// nested comp's length is on the comp, and a footage item's comes from the
/// media probe, so only a caller holding the project can answer.
///
/// `None` — a source this document no longer has, or a file that will not probe
/// — is the honest "no reach", and that is exactly how the reach is drawn.
#[frb(ignore)]
fn clip_source_duration(
    state: &LumitBridgeState,
    doc: &lumit_core::Document,
    source: lumit_core::sequence::ClipSource,
) -> Option<lumit_core::time::Rational> {
    match source {
        lumit_core::sequence::ClipSource::Comp(id) => Some(doc.comp(id)?.duration.0),
        lumit_core::sequence::ClipSource::Footage(id) => {
            let lumit_core::model::ProjectItem::Footage(footage) = doc.item(id)? else {
                return None;
            };
            let _path = FootageReference::resolve_path(state, footage)?;
            #[cfg(not(feature = "media"))]
            {
                // No decoder, so no honest length — and a guess here would put
                // a ghost on the row that no build with a decoder agrees with.
                None
            }
            #[cfg(feature = "media")]
            {
                let probed = crate::probe::ensure_probed(&_path)?;
                // The only sanctioned route back from the container's
                // floating-point duration is an explicit grid
                // (docs/impl/rational-time.md §4) — the same millisecond grid
                // `media_info` reports on, so the two cannot disagree.
                lumit_core::time::Rational::from_f64_on_grid(probed.duration_seconds, 1000).ok()
            }
        }
    }
}

/// One clip as the Timeline draws it. The one place the mapping lives: the
/// comp read model, `get_info` and `get_clips` all build clips through here,
/// and three copies of it is three chances for a clip to sit somewhere
/// different depending on which read found it.
#[frb(ignore)]
fn bridge_clip(
    comp: &lumit_core::model::Composition,
    layer: &Layer,
    clip: &lumit_core::sequence::Clip,
    source_duration: Option<lumit_core::time::Rational>,
) -> BridgeClip {
    use lumit_core::time::CompTime;
    // A clip's `place_*` are in layer time; every frame below carries the
    // layer's own zero already added.
    let at = |t: lumit_core::time::Rational| {
        comp.frame_rate
            .frame_at(CompTime(layer.start_offset.0.checked_add(t).unwrap_or(t)))
    };
    let reach = clip.source_reach(source_duration);
    BridgeClip {
        id: clip.id,
        place_start: rational_of(clip.place_start),
        place_duration: rational_of(clip.place_duration),
        start_frame: at(clip.place_start),
        end_frame: at(clip.place_end()),
        speed_percent: clip.constant_speed().map(|s| s * 100.0),
        retimed: clip.retime.is_some(),
        // Keyed in clip time, so it crosses with no offset applied — unlike a
        // layer's, which the bridge carries out by the layer's own zero
        // (K-213). A clip's zero *is* its start.
        retime: BridgeScalar::read_at(&clip.effective_retime(), lumit_core::time::Rational::ZERO),
        reach_start_frame: reach.map(|(start, _)| at(start)),
        reach_end_frame: reach.map(|(_, end)| at(end)),
    }
}

/// Everything the Timeline outline, its bars, and the Hierarchy draw for one
/// layer, in one crossing (K-183). Read one getter at a time this cost
/// seven-plus bridge calls per row per rebuild — each cloning the composition
/// out of the snapshot — plus two time↔frame trips per bar.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLayerInfo {
    pub name: String,
    pub kind: BridgeLayerKind,
    pub switches: BridgeLayerSwitches,
    /// Blend mode as an index into `list_blend_modes`.
    pub blend: u32,
    pub span: BridgeSpan,
    /// The span at the comp's own rate, so drawing needs no time↔frame trips.
    pub in_frame: i64,
    pub out_frame: i64,
    /// Sequence clip starts as comp frames (empty on other kinds) — what the
    /// bar draws its split lines from.
    pub clip_frames: Vec<i64>,
    /// Every clip on a Sequence layer, in list order (empty on other kinds) —
    /// what the expanded sequence view draws (K-248).
    ///
    /// In the read model rather than fetched per clip, so opening a Sequence
    /// layer costs no bridge calls at all (K-184).
    pub clips: Vec<BridgeClip>,
    pub parent: Option<Uuid>,
    /// The parent layer's current name, so the outline's parent picker renders
    /// with no second lookup. None when there is no parent, or it is dangling.
    pub parent_name: Option<String>,
    /// The whole transform, one scalar per property (K-184).
    pub transform: BridgeTransform,
    /// How each two-axis property is shown (K-571) — which decides how many
    /// rows the Transform group has, so it is read here with the rest of the
    /// drawing data rather than asked for per row.
    pub axis_modes: BridgeAxisModes,
    /// Every effect on the layer, with every parameter's value (K-184). Plain
    /// data for *drawing*; an edit reads fresh instance handles at commit time.
    pub effects: Vec<crate::api::effect::BridgeEffectInstanceInfo>,
    /// The label colour index into the theme's palette, drawn as the outline's
    /// swatch. Out-of-range values wrap rather than fault.
    pub label: u8,
    /// The layer's matte, for the outline's matte cell (K-184: the row draws
    /// with no bridge calls). Writes still go through `set_matte`.
    pub matte: Option<BridgeMatte>,
    /// The Retime property (K-197), or None when the layer is not retimed —
    /// which is exactly what decides whether the fold-out shows a Retime row.
    pub retime: Option<BridgeScalar>,
    /// The layer's masks (K-222), bottom of the stack first. Carried in the
    /// read model for the same reason the effects are: the Timeline's
    /// twirl-down draws a row per mask, and asking per row per frame is the
    /// cost K-184 exists to remove. Edits still go through `set_mask`.
    pub masks: Vec<BridgeMask>,
    /// The layer's paint strokes (K-227), oldest first — carried for the same
    /// reason the masks are: the Timeline lists them, and the Viewer needs to
    /// know a layer has some without asking per frame.
    pub paint: Vec<BridgeStroke>,
    /// A shape layer's art (K-237), bottom first; empty on every other kind.
    /// Carried for the same reason again — and for one more: the art *is* the
    /// layer's size, so the Viewer's wireframe reads it here.
    pub shape_contents: Vec<BridgeShapeItem>,
    /// The layer's own markers (K-254), and the comp frame each falls on — the
    /// marker's layer-local time carried out by the layer's start offset — so
    /// the bar needs no time↔frame trip to draw one. In the read model because
    /// the Timeline draws them on every rebuild, which is the cost K-184 exists
    /// to remove.
    pub markers: Vec<BridgeLayerMarker>,
    /// Whether optical flow is live on this layer (K-088/K-331) — the switch
    /// cluster's Flow cell, and what decides whether the fold-out shows a Flow
    /// group. In the read model because the Timeline draws that cell on every
    /// rebuild, and asking per row per frame is exactly the cost K-184 removed.
    pub flow: bool,
    /// The Flow group's Input rate (K-095/K-160), the one animatable member —
    /// carried here so its fold-out row can draw its keyframe diamonds without
    /// a call, exactly as the Retime row's scalar is.
    pub flow_input_rate: BridgeScalar,
    /// **Edited since track** (K-578): this solve-linked Camera layer carries a
    /// correction, or — on a tracked layer — a camera that follows it does.
    ///
    /// One fact from where the user stands ("the tracked motion has been
    /// nudged"), read from two sides because it is drawn on two rows: beside
    /// the camera's link badge, and on the Camera track effect's card. In the
    /// read model rather than as a call, because both rows are rebuilt on every
    /// document revision and a call there is exactly the cost K-184 removed.
    pub track_corrected: bool,
    /// A Text layer's animator groups (K-609), in order; empty on every other
    /// kind. Carried for the same reason the masks and the strokes are: the
    /// Timeline draws a row per animator property and the graph editor reads
    /// its curves from here, and asking per row per frame is exactly the cost
    /// K-184 exists to remove. Edits still go through `set_text`.
    pub text_animators: Vec<crate::api::assets::BridgeTextAnimator>,
}

/// One marker on a layer's bar: the marker itself plus where it lands at the
/// comp's rate, worked out here so drawing costs nothing across the seam.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLayerMarker {
    pub marker: BridgeMarker,
    pub frame: i64,
}

/// Build one layer's [`BridgeLayerInfo`] from an already-fetched composition —
/// the shared body of [`LayerReference::get_info`] and the comp-wide
/// [`crate::api::composition::CompositionReference::get_model`] (K-184).
#[frb(ignore)]
pub(crate) fn read_layer_info(
    state: &LumitBridgeState,
    doc: &lumit_core::Document,
    comp: &lumit_core::model::Composition,
    layer: &Layer,
) -> BridgeLayerInfo {
    use lumit_core::model::LayerKind as K;
    let clip_frames = match &layer.kind {
        K::Sequence { clips } => clips
            .iter()
            .map(|c| {
                comp.frame_rate
                    .frame_at(lumit_core::time::CompTime(c.place_start))
            })
            .collect(),
        _ => Vec::new(),
    };
    let clips = match &layer.kind {
        K::Sequence { clips } => clips
            .iter()
            .map(|c| bridge_clip(comp, layer, c, clip_source_duration(state, doc, c.source)))
            .collect(),
        _ => Vec::new(),
    };
    BridgeLayerInfo {
        name: layer.name.clone(),
        kind: bridge_kind(layer),
        switches: bridge_switches(layer),
        blend: lumit_core::model::BlendMode::ALL
            .iter()
            .position(|b| *b == layer.blend)
            .unwrap_or(0) as u32,
        span: BridgeSpan {
            in_point: rational_of(layer.in_point.0),
            out_point: rational_of(layer.out_point.0),
            start_offset: rational_of(layer.start_offset.0),
        },
        in_frame: comp.frame_rate.frame_at(layer.in_point),
        out_frame: comp.frame_rate.frame_at(layer.out_point),
        clip_frames,
        clips,
        parent: layer.parent,
        parent_name: layer.parent.and_then(|p| {
            comp.layers
                .iter()
                .find(|l| l.id == p)
                .map(|l| l.name.clone())
        }),
        transform: BridgeTransform::read_at(&layer.transform, layer.start_offset.0),
        axis_modes: BridgeAxisModes::of(layer.transform.axis_modes),
        effects: layer
            .effects
            .iter()
            .map(|e| crate::api::effect::read_instance_info(e, layer.start_offset.0))
            .collect(),
        label: layer.label,
        matte: layer.matte.as_ref().map(|m| BridgeMatte {
            layer: m.layer,
            luma: matches!(m.channel, lumit_core::model::MatteChannel::Luma),
            inverted: m.inverted,
        }),
        retime: layer
            .retime
            .as_ref()
            .map(|r| BridgeScalar::read_at(r, layer.start_offset.0)),
        masks: layer
            .masks
            .iter()
            .map(|m| BridgeMask::read_at(m, layer.start_offset.0))
            .collect(),
        paint: layer
            .paint
            .iter()
            .map(|s| BridgeStroke::read_at(s, layer.start_offset.0))
            .collect(),
        markers: layer
            .markers
            .iter()
            .map(|m| BridgeLayerMarker {
                marker: bridge_marker(m, comp.frame_rate),
                // A layer marker's time is the layer's own, so where it lands
                // on the comp is that time carried out by the layer's start
                // offset — exactly as a Sequence clip's is. That is what makes
                // markers travel when the layer is dragged along the timeline.
                frame: comp.frame_rate.frame_at(CompTime(
                    layer
                        .start_offset
                        .0
                        .checked_add(m.time.0)
                        .unwrap_or(m.time.0),
                )),
            })
            .collect(),
        shape_contents: match &layer.kind {
            lumit_core::model::LayerKind::Shape { contents } => contents
                .iter()
                .map(|i| BridgeShapeItem::read_at(i, layer.start_offset.0))
                .collect(),
            _ => Vec::new(),
        },
        flow: matches!(
            layer.interpolation,
            lumit_core::retime::Interpolation::Flow(_)
        ),
        flow_input_rate: BridgeScalar::read_at(
            match &layer.interpolation {
                lumit_core::retime::Interpolation::Flow(p) => &p.input_fps,
                _ => &ZERO_RATE,
            },
            layer.start_offset.0,
        ),
        // The camera's own lane, or — on a tracked layer — any camera in this
        // comp that follows it. The scan happens only for a layer wearing a
        // Camera track, which is one layer in a comp at most times and none in
        // most comps, so the ordinary read model pays a `matches!` and nothing.
        track_corrected: lumit_core::track::has_correction(layer)
            || (lumit_core::track::wears_camera_track(layer)
                && comp.layers.iter().any(|l| {
                    matches!(
                        l.kind,
                        lumit_core::model::LayerKind::Camera {
                            solve_link: Some(tracked),
                            ..
                        } if tracked == layer.id
                    ) && lumit_core::track::has_correction(l)
                })),
        text_animators: match &layer.kind {
            K::Text { document } => document
                .animators
                .iter()
                .map(|a| crate::api::assets::read_animator(a, layer.start_offset.0))
                .collect(),
            _ => Vec::new(),
        },
    }
}

/// A shared Auto rate for layers with no flow, so the read model always has a
/// scalar to hand back without allocating one per layer per rebuild.
static ZERO_RATE: std::sync::LazyLock<lumit_core::anim::Property> =
    std::sync::LazyLock::new(lumit_core::anim::Property::zero);

/// The most buckets one peak query may ask for (K-280). A lane asks for a
/// bucket per pixel column, and no panel is four thousand columns wide on any
/// display this ships to; the cap is what stops a frontend bug turning into an
/// unbounded allocation across the seam (docs/14 §5).
pub const MAX_PEAK_BUCKETS: u32 = 4096;

/// One window of a source's waveform, summarised to exactly the buckets the
/// lane asked for (K-280).
///
/// The **window** is the point: a lane asks for the stretch of audio it is
/// currently showing at the number of buckets it has pixel columns, so the
/// drawn detail follows the zoom instead of being fixed at import. Buckets that
/// fall outside the audio come back silent rather than missing, so a caller's
/// column index and a bucket index always agree.
///
/// One to four **bands** ride in the same answer. A single-wave lane asks for
/// one (the whole signal); a multiwave lane asks for three (bass, middle,
/// treble) and stacks them, which is what shows the difference between a kick
/// and a hi-hat inside a loud passage that is otherwise one solid block.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAudioPeaks {
    /// How long the whole source runs, so a lane can tell where its window sits.
    pub duration_seconds: f64,
    /// The window these buckets span, in the caller's own clock — source
    /// seconds for a layer, clip-local seconds for a clip.
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// How many bands are stacked here: 1 (the whole signal) or 3 (bass,
    /// middle, treble, in that order).
    pub bands: u32,
    /// Buckets per band.
    pub buckets: u32,
    /// Band-major triples: band `b`'s bucket `i` is `min`, `max`, `rms` at
    /// `3 * (b * buckets + i)`, each in −1..1.
    pub values: Vec<f32>,
}

impl BridgeAudioPeaks {
    /// The answer for a source with nothing to draw: no audio, no media
    /// feature, a file that has gone missing. A lane draws it as an empty lane.
    #[frb(ignore)]
    fn empty() -> BridgeAudioPeaks {
        BridgeAudioPeaks {
            duration_seconds: 0.0,
            start_seconds: 0.0,
            end_seconds: 0.0,
            bands: 0,
            buckets: 0,
            values: Vec::new(),
        }
    }

    /// The bands a `multiwave` flag asks for, in the order they stack.
    #[frb(ignore)]
    #[cfg(feature = "media")]
    fn bands_of(multiwave: bool) -> Vec<lumit_audio::peaks::Band> {
        if multiwave {
            lumit_audio::peaks::Band::stack().to_vec()
        } else {
            vec![lumit_audio::peaks::Band::Full]
        }
    }
}

/// A layer used as another layer's matte (docs/03 §5.1).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeMatte {
    pub layer: Uuid,
    /// Whether the matte reads the source's alpha or its luminance.
    pub luma: bool,
    pub inverted: bool,
}

/// A layer's whole transform, one scalar per property.
///
/// Read as a group rather than a property at a time because the panel draws them
/// as a group and a drag on one axis previews the others unchanged — eleven
/// round trips per frame to rebuild what one call already has would be the
/// snapshot habit creeping back in. Writing is per-property (see
/// [`LayerReference::set_transform`]), which is what keeps each edit exactly
/// invertible.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTransform {
    pub anchor_x: BridgeScalar,
    pub anchor_y: BridgeScalar,
    pub position_x: BridgeScalar,
    pub position_y: BridgeScalar,
    /// The 2.5D depth (K-023). Present on every layer; only meaningful, and only
    /// drawn, when the layer's 3D switch is on.
    pub position_z: BridgeScalar,
    /// Percent, 100 = natural size.
    pub scale_x: BridgeScalar,
    pub scale_y: BridgeScalar,
    /// Degrees, about z — the 2D rotation.
    pub rotation: BridgeScalar,
    pub rotation_x: BridgeScalar,
    pub rotation_y: BridgeScalar,
    /// Percent, 0..100.
    pub opacity: BridgeScalar,
}

/// Which transform property an edit names ([`lumit_core::model::TransformProp`]).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTransformProp {
    AnchorX,
    AnchorY,
    PositionX,
    PositionY,
    PositionZ,
    ScaleX,
    ScaleY,
    Rotation,
    RotationX,
    RotationY,
    Opacity,
}

/// Which two-axis transform property an axis-mode edit names (K-571,
/// [`lumit_core::model::TransformPair`]).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTransformPair {
    Anchor,
    Position,
    Scale,
}

/// How one two-axis property is shown and edited (K-571,
/// [`lumit_core::model::AxisMode`]).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAxisMode {
    /// One row, one box, the x:y ratio held on every edit.
    Linked,
    /// One row, a box per axis, one stopwatch over all of them.
    Combined,
    /// A row per axis, each with its own stopwatch and its own curve.
    Separated,
}

/// Every pair's mode, carried in the read model so the panels can draw the
/// right rows with no bridge call (K-184).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeAxisModes {
    pub anchor: BridgeAxisMode,
    pub position: BridgeAxisMode,
    pub scale: BridgeAxisMode,
}

impl BridgeTransformPair {
    #[frb(ignore)]
    pub(crate) fn core(self) -> lumit_core::model::TransformPair {
        use lumit_core::model::TransformPair as P;
        match self {
            BridgeTransformPair::Anchor => P::Anchor,
            BridgeTransformPair::Position => P::Position,
            BridgeTransformPair::Scale => P::Scale,
        }
    }
}

impl BridgeAxisMode {
    #[frb(ignore)]
    pub(crate) fn core(self) -> lumit_core::model::AxisMode {
        use lumit_core::model::AxisMode as M;
        match self {
            BridgeAxisMode::Linked => M::Linked,
            BridgeAxisMode::Combined => M::Combined,
            BridgeAxisMode::Separated => M::Separated,
        }
    }

    #[frb(ignore)]
    pub(crate) fn of(mode: lumit_core::model::AxisMode) -> Self {
        use lumit_core::model::AxisMode as M;
        match mode {
            M::Linked => BridgeAxisMode::Linked,
            M::Combined => BridgeAxisMode::Combined,
            M::Separated => BridgeAxisMode::Separated,
        }
    }
}

impl BridgeAxisModes {
    #[frb(ignore)]
    pub(crate) fn of(modes: lumit_core::model::AxisModes) -> Self {
        Self {
            anchor: BridgeAxisMode::of(modes.anchor),
            position: BridgeAxisMode::of(modes.position),
            scale: BridgeAxisMode::of(modes.scale),
        }
    }
}

/// A document rational as the integer pair the bridge carries.
#[frb(ignore)]
fn rational_of(r: lumit_core::time::Rational) -> BridgeRational {
    BridgeRational {
        num: r.num(),
        den: r.den(),
    }
}

/// The inverse. A zero or negative denominator is refused rather than
/// normalised: it means the caller built a time wrongly, and quietly fixing it
/// would put a span somewhere nobody asked for.
#[frb(ignore)]
fn comp_time(r: BridgeRational) -> Result<lumit_core::time::Rational, BridgeError> {
    lumit_core::time::Rational::new(r.num, r.den).map_err(|_| BridgeError::InvalidTime)
}

impl BridgeTransformProp {
    #[frb(ignore)]
    pub(crate) fn core(self) -> lumit_core::model::TransformProp {
        use lumit_core::model::TransformProp as P;
        match self {
            BridgeTransformProp::AnchorX => P::AnchorX,
            BridgeTransformProp::AnchorY => P::AnchorY,
            BridgeTransformProp::PositionX => P::PositionX,
            BridgeTransformProp::PositionY => P::PositionY,
            BridgeTransformProp::PositionZ => P::PositionZ,
            BridgeTransformProp::ScaleX => P::ScaleX,
            BridgeTransformProp::ScaleY => P::ScaleY,
            BridgeTransformProp::Rotation => P::Rotation,
            BridgeTransformProp::RotationX => P::RotationX,
            BridgeTransformProp::RotationY => P::RotationY,
            BridgeTransformProp::Opacity => P::Opacity,
        }
    }
}

impl BridgeTransform {
    #[frb(ignore)]
    /// `offset` is the layer's `start_offset`: keys cross on the composition's
    /// clock, not the layer's own (K-213).
    #[allow(clippy::similar_names)]
    pub(crate) fn read_at(
        group: &lumit_core::model::TransformGroup,
        offset: Rational,
    ) -> BridgeTransform {
        BridgeTransform {
            anchor_x: BridgeScalar::read_at(&group.anchor_x, offset),
            anchor_y: BridgeScalar::read_at(&group.anchor_y, offset),
            position_x: BridgeScalar::read_at(&group.position_x, offset),
            position_y: BridgeScalar::read_at(&group.position_y, offset),
            position_z: BridgeScalar::read_at(&group.position_z, offset),
            scale_x: BridgeScalar::read_at(&group.scale_x, offset),
            scale_y: BridgeScalar::read_at(&group.scale_y, offset),
            rotation: BridgeScalar::read_at(&group.rotation, offset),
            rotation_x: BridgeScalar::read_at(&group.rotation_x, offset),
            rotation_y: BridgeScalar::read_at(&group.rotation_y, offset),
            opacity: BridgeScalar::read_at(&group.opacity, offset),
        }
    }

    /// Write this whole group onto `target`, for the drag preview — which needs
    /// a document to render, not an op to commit.
    #[frb(ignore)]
    pub(crate) fn write_at(
        &self,
        target: &mut lumit_core::model::TransformGroup,
        offset: Rational,
    ) -> Result<(), BridgeError> {
        target.anchor_x.animation = self.anchor_x.animation_at(offset)?;
        target.anchor_y.animation = self.anchor_y.animation_at(offset)?;
        target.position_x.animation = self.position_x.animation_at(offset)?;
        target.position_y.animation = self.position_y.animation_at(offset)?;
        target.position_z.animation = self.position_z.animation_at(offset)?;
        target.scale_x.animation = self.scale_x.animation_at(offset)?;
        target.scale_y.animation = self.scale_y.animation_at(offset)?;
        target.rotation.animation = self.rotation.animation_at(offset)?;
        target.rotation_x.animation = self.rotation_x.animation_at(offset)?;
        target.rotation_y.animation = self.rotation_y.animation_at(offset)?;
        target.opacity.animation = self.opacity.animation_at(offset)?;
        Ok(())
    }
}

// Three ids and nothing else, so a copy is as good as the original — which is
// what lets a caller pass the same reference to a list and keep using it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[frb]
pub struct LayerReference {
    #[frb(name = "internalprojectId")]
    pub project_id: Uuid,

    #[frb(name = "internalcompId")]
    pub comp_id: Uuid,

    #[frb(name = "internallayerId")]
    pub layer_id: Uuid,
}

impl LayerReference {
    #[frb(ignore)]
    pub fn new(project_id: Uuid, comp_id: Uuid, layer_id: Uuid) -> LayerReference {
        LayerReference {
            project_id,
            comp_id,
            layer_id,
        }
    }

    #[frb(ignore)]
    pub fn project_id(&self) -> Uuid {
        self.project_id
    }

    #[frb(ignore)]
    pub fn comp_id(&self) -> Uuid {
        self.comp_id
    }

    #[frb(ignore)]
    pub fn id(&self) -> Uuid {
        self.layer_id
    }

    #[frb(ignore)]
    pub(crate) fn project(&self) -> Result<Arc<std::sync::RwLock<LumitBridgeState>>, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects.get(&self.project_id);

        let p = project.ok_or(BridgeError::InvalidProject)?;
        Ok(p.clone())
    }

    /// The composition this layer lives in, cloned out of the current snapshot.
    /// The read lock is released by the time it returns, so a caller is free to
    /// take the write lock next.
    #[frb(ignore)]
    pub(crate) fn composition(&self) -> Result<lumit_core::model::Composition, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();

        match snapshot
            .item(self.comp_id)
            .ok_or(BridgeError::InvalidItem)?
        {
            lumit_core::model::ProjectItem::Composition(composition) => Ok(composition.clone()),
            _ => Err(BridgeError::InvalidItem),
        }
    }

    #[frb(ignore)]
    pub(crate) fn item(&self) -> Result<Layer, BridgeError> {
        self.composition()?
            .layers
            .into_iter()
            .find(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)
    }

    #[frb(sync)]
    pub fn equals(&self, layer: &LayerReference) -> bool {
        self.comp_id == layer.comp_id
            && self.project_id == layer.project_id
            && self.layer_id == layer.layer_id
    }

    #[frb(sync)]
    pub fn get_name(&self) -> Result<String, BridgeError> {
        let item = self.item()?;

        Ok(item.name)
    }

    /// One read for everything a row draws — see [`BridgeLayerInfo`]. One
    /// document lock and one crossing, where the per-field getters cost one of
    /// each per field.
    #[frb(sync)]
    pub fn get_info(&self) -> Result<BridgeLayerInfo, BridgeError> {
        let proj = self.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Composition(comp)) = doc.item(self.comp_id) else {
            return Err(BridgeError::InvalidItem);
        };
        let layer = comp
            .layers
            .iter()
            .find(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;
        Ok(read_layer_info(&state, &doc, comp, layer))
    }

    #[frb(sync)]
    pub fn rename(&self, name: String) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;

        proj.store
            .commit(lumit_core::Op::RenameLayer {
                comp: self.comp_id,
                layer: self.layer_id,
                name,
            })
            .map_err(BridgeError::OpError)?;

        Ok(())
    }

    /// This layer as text, for [`crate::api::composition::CompositionReference::
    /// paste_layer`] — the clipboard's payload (K-275).
    ///
    /// The whole layer: its kind and source, its transform with every keyframe,
    /// its masks, paint, effects, switches, markers and retime. It is the
    /// document's own `Layer`, serialised, so anything the file format carries
    /// travels — including fields a newer Lumit added, which ride in the
    /// `extra` maps exactly as they do through a save (docs/10 §1.1).
    ///
    /// A *reference* it holds to another layer — its parent, its track matte —
    /// is copied as it stands and resolved at the paste, which is the only end
    /// that knows whether the layer being pointed at is there.
    #[frb(sync)]
    pub fn copy_layer(&self) -> Result<String, BridgeError> {
        let layer = self.item()?;
        serde_json::to_string(&serde_json::json!({
            "format": 1,
            "kind": "layer",
            // Informational: which comp it left, so a paste into that same comp
            // can keep a parent or matte reference that would otherwise dangle.
            "comp": self.comp_id,
            "layer": layer,
        }))
        .map_err(|_| BridgeError::InvalidItem)
    }

    /// This layer's effects as a `.lumfx` document, for [`Self::paste_effects`]
    /// (K-275). `effects` copies those; an empty list copies the whole stack.
    ///
    /// A list rather than one id (K-300), because an effect selection can hold
    /// several — and they come out in **stack order**, not in the order they
    /// were picked, so a copied group pastes back in the order it was drawn in.
    /// Ids that name nothing on this layer are ignored; naming none of them at
    /// all is [`BridgeError::InvalidEffect`] rather than a silent whole-stack
    /// copy.
    ///
    /// Deliberately the **same document a preset is**, so an effect copied from
    /// one layer can be saved as a preset and a preset can be pasted as an
    /// effect — one shape, not two that drift.
    #[frb(sync)]
    pub fn copy_effects(&self, effects: Vec<Uuid>) -> Result<String, BridgeError> {
        let stack = self.item()?.effects;
        let taken: Vec<_> = if effects.is_empty() {
            stack
        } else {
            stack
                .into_iter()
                .filter(|e| effects.contains(&e.id))
                .collect()
        };
        if taken.is_empty() {
            return Err(BridgeError::InvalidEffect);
        }
        let name = taken
            .first()
            .map(|e| e.effect.match_name.clone())
            .unwrap_or_default();
        lumit_core::preset::to_json(&name, &taken).map_err(|_| BridgeError::InvalidPreset)
    }

    /// Append copied effects to this layer's stack, **timed to the playhead**
    /// (K-275): whatever the earliest keyframe among them was, it lands at
    /// `at_frame` and the rest keep their spacing.
    ///
    /// The owner's rule, and the one that makes a copied animation useful: an
    /// effect copied from a layer that flashes at 4 s and pasted while the
    /// playhead sits at 12 s flashes at 12 s, not off the end of the comp.
    /// Effects with no keyframes at all paste unchanged — there is no timing to
    /// place. Each arrives with a fresh instance id, exactly as a preset does.
    ///
    /// `at_frame` is a **comp** frame; the shift is worked out in the target
    /// layer's own local time, so pasting onto a layer that starts later does
    /// not double-count its offset.
    #[frb(sync)]
    pub fn paste_effects(&self, text: String, at_frame: i64) -> Result<(), BridgeError> {
        let preset =
            lumit_core::preset::from_json(&text).map_err(|_| BridgeError::InvalidPreset)?;
        let mut fresh = lumit_core::preset::instantiated(&preset);

        // Comp frame -> the layer's own clock, the space keyframes live in.
        let comp = self.composition()?;
        let layer = self.item()?;
        let at_comp = comp
            .frame_rate
            .time_of_frame(at_frame.max(0))
            .map_err(|_| BridgeError::InvalidTime)?;
        let at_local = at_comp
            .delta(layer.start_offset)
            .map_err(|_| BridgeError::InvalidTime)?;

        if let Some(first) = lumit_core::preset::first_key_time(&fresh) {
            let delta = at_local
                .0
                .checked_sub(first)
                .map_err(|_| BridgeError::InvalidTime)?;
            lumit_core::preset::shift_keys(&mut fresh, delta);
        }

        self.with_effects(move |effects| {
            effects.extend(fresh);
            Ok(())
        })
    }

    /// Serialise this layer's whole effect stack to `.lumfx` JSON.
    ///
    /// Returns the text rather than writing a file: choosing where something
    /// goes is the file picker's job, and the engine has no business opening one.
    /// A layer with no effects still saves — an empty preset is a valid, if
    /// unexciting, document.
    #[frb(sync)]
    pub fn save_preset(&self, name: String) -> Result<String, BridgeError> {
        let effects = self.item()?.effects;
        serde_json::to_string_pretty(&serde_json::json!({
            "format": 1,
            "name": name,
            "effects": effects,
        }))
        .map_err(|_| BridgeError::InvalidPreset)
    }

    /// Append a `.lumfx` preset's effects to this layer's stack, as one op.
    ///
    /// Each arrives with a **fresh** instance id (K-065): applying one preset to
    /// several layers must not give them effects that share an id, since an id
    /// is instance identity and every op that names an effect uses it.
    ///
    /// A document written by a newer Lumit still loads — unknown fields ride
    /// along in each effect's `extra` map, exactly as the project file tolerates
    /// additions. Only text that is not a preset at all is refused.
    #[frb(sync)]
    pub fn load_preset(&self, text: String) -> Result<(), BridgeError> {
        #[derive(serde::Deserialize)]
        struct Preset {
            effects: Vec<EffectInstance>,
        }

        let preset: Preset = serde_json::from_str(&text).map_err(|_| BridgeError::InvalidPreset)?;
        let fresh: Vec<EffectInstance> = preset
            .effects
            .into_iter()
            .map(|mut effect| {
                effect.id = Uuid::now_v7();
                effect
            })
            .collect();

        self.with_effects(move |effects| {
            effects.extend(fresh);
            Ok(())
        })
    }

    /// This layer's masks, bottom of the stack first (K-222).
    ///
    /// Empty on a layer with none, which is most layers — the Timeline asks
    /// every row whether it has masks to list, exactly as it asks about clips.
    #[frb(sync)]
    pub fn get_masks(&self) -> Result<Vec<BridgeMask>, BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        Ok(layer
            .masks
            .iter()
            .map(|m| BridgeMask::read_at(m, offset))
            .collect())
    }

    /// Add `mask` to the top of this layer's stack.
    ///
    /// The whole list is committed, because that is the op the engine has and
    /// it is exactly invertible (`SetLayerMasks`) — an add, a delete and a
    /// reorder are all one shape of edit, and each is one undo step.
    ///
    /// A path of fewer than two vertices is refused: it is not a shape, and a
    /// mask that gates nothing would be a row in the Timeline with nothing
    /// behind it.
    #[frb(sync)]
    pub fn add_mask(&self, mask: BridgeMask) -> Result<(), BridgeError> {
        if mask.vertices.len() < 2 {
            return Err(BridgeError::EmptyPath);
        }
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut masks = layer.masks;
        masks.push(mask.write(offset)?);
        self.commit_masks(masks)
    }

    /// Replace one mask — its path, its name, its invert switch, its opacity.
    /// Named by id, so a stale reference is a calm error rather than an edit
    /// landing on whichever mask happens to sit at that index now.
    /// `at` is the playhead, in composition time. It matters only for a mask
    /// whose **shape** is keyed, where it decides which key the dragged
    /// vertices land on; see [`BridgeMask::write_over`].
    #[frb(sync)]
    pub fn set_mask(
        &self,
        mask: BridgeMask,
        at: Option<BridgeRational>,
    ) -> Result<(), BridgeError> {
        if mask.vertices.len() < 2 {
            return Err(BridgeError::EmptyPath);
        }
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut masks = layer.masks;
        let at_index = masks
            .iter()
            .position(|m| m.id == mask.id)
            .ok_or(BridgeError::NoSuchMask)?;
        // Patched over the mask it replaces, not built fresh: an edit to a
        // mask's opacity must not throw away its path keyframes.
        let when = match at {
            Some(t) => {
                Some(Rational::new(t.num, t.den).map_err(|_| BridgeError::InvalidKeyframes)?)
            }
            None => None,
        };
        masks[at_index] = mask.write_over(&masks[at_index], offset, when)?;
        self.commit_masks(masks)
    }

    /// Remove a mask by id.
    #[frb(sync)]
    pub fn delete_mask(&self, id: Uuid) -> Result<(), BridgeError> {
        let mut masks = self.item()?.masks;
        let before = masks.len();
        masks.retain(|m| m.id != id);
        if masks.len() == before {
            return Err(BridgeError::NoSuchMask);
        }
        self.commit_masks(masks)
    }

    /// Key this mask's **shape** at `time`, or take the key already there away
    /// (K-339, K-340) — the ◆ on the mask's Path row.
    ///
    /// A planted key holds the shape the mask is *already showing* at that
    /// moment, so pressing ◆ never moves anything: on an unanimated mask that
    /// is its static path, and on an animated one it is what the shapes either
    /// side interpolate to. Planting the first key is what starts the shape
    /// animating.
    ///
    /// `time` is composition time, as every other keyframe time that crosses
    /// here; the layer's own offset is taken back off inside.
    #[frb(sync)]
    pub fn toggle_mask_path_key(&self, id: Uuid, time: BridgeRational) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut masks = layer.masks;
        let at = masks
            .iter()
            .position(|m| m.id == id)
            .ok_or(BridgeError::NoSuchMask)?;
        let time = Rational::new(time.num, time.den)
            .map_err(|_| BridgeError::InvalidKeyframes)?
            .checked_sub(offset)
            .map_err(|_| BridgeError::InvalidKeyframes)?;
        let mask = &mut masks[at];
        // Compared by value: a key planted here and one loaded from a file are
        // the same key, and `Rational` reduces, so equality is exact.
        if let Some(i) = mask.path_keys.iter().position(|k| k.time == time) {
            mask.path_keys.remove(i);
        } else {
            let path = mask.path_at(time.to_f64()).into_owned();
            let key = lumit_core::mask::PathKeyframe {
                time,
                path,
                interp_in: lumit_core::anim::SideInterp::Linear,
                interp_out: lumit_core::anim::SideInterp::Linear,
            };
            let at = mask
                .path_keys
                .iter()
                .position(|k| k.time > time)
                .unwrap_or(mask.path_keys.len());
            mask.path_keys.insert(at, key);
        }
        self.commit_masks(masks)
    }

    /// Drag one of the shape's keys along the timeline (K-340) — the lane
    /// diamond, which moves a path key exactly as it moves a scalar's.
    ///
    /// Refused, with `false`, when the move would land on or step over a
    /// neighbour: keys are sorted with unique times and the evaluator walks
    /// them assuming so, and a drag that would break the order simply leaves
    /// the key where it was rather than reordering under the pointer.
    #[frb(sync)]
    pub fn move_mask_path_key(
        &self,
        id: Uuid,
        from: BridgeRational,
        to: BridgeRational,
    ) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut masks = layer.masks;
        let at = masks
            .iter()
            .position(|m| m.id == id)
            .ok_or(BridgeError::NoSuchMask)?;
        let local = |t: BridgeRational| -> Result<Rational, BridgeError> {
            Rational::new(t.num, t.den)
                .map_err(|_| BridgeError::InvalidKeyframes)?
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)
        };
        let (from, to) = (local(from)?, local(to)?);
        let mask = &mut masks[at];
        let Some(i) = mask.path_keys.iter().position(|k| k.time == from) else {
            return Ok(false);
        };
        for (j, key) in mask.path_keys.iter().enumerate() {
            if j == i {
                continue;
            }
            if (j < i && key.time >= to) || (j > i && key.time <= to) {
                return Ok(false);
            }
        }
        mask.path_keys[i].time = to;
        self.commit_masks(masks)?;
        Ok(true)
    }

    /// Re-time and re-ease this mask's shape keys in one write (K-344) — what
    /// the graph editor commits when a handle is dragged, and what a lane drag
    /// of several keys at once needs.
    ///
    /// `keys` must name every key the mask has, in order; their `value` is
    /// ignored, because a path key holds a shape rather than a number. Refused
    /// as a whole if the times are not strictly ascending: the evaluator walks
    /// the list assuming they are, and a half-applied reorder is not a mask.
    #[frb(sync)]
    pub fn set_mask_path_keys(
        &self,
        id: Uuid,
        keys: Vec<BridgeKeyframe>,
    ) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut masks = layer.masks;
        let at = masks
            .iter()
            .position(|m| m.id == id)
            .ok_or(BridgeError::NoSuchMask)?;
        if keys.len() != masks[at].path_keys.len() {
            return Ok(false);
        }
        let mut written = Vec::with_capacity(keys.len());
        for (key, existing) in keys.iter().zip(masks[at].path_keys.iter()) {
            let time = Rational::new(key.time.num, key.time.den)
                .map_err(|_| BridgeError::InvalidKeyframes)?
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)?;
            if written
                .last()
                .is_some_and(|p: &lumit_core::mask::PathKeyframe| time <= p.time)
            {
                return Ok(false);
            }
            written.push(lumit_core::mask::PathKeyframe {
                time,
                path: existing.path.clone(),
                interp_in: key.interp_in.write(),
                interp_out: key.interp_out.write(),
            });
        }
        masks[at].path_keys = written;
        self.commit_masks(masks)?;
        Ok(true)
    }

    /// Stop the shape animating, keeping the shape it shows at `time` (K-340).
    ///
    /// The stopwatch turning off, and it matches what the stopwatch does
    /// everywhere else: the value that stays is the one the curve reads *at the
    /// playhead*, not the first key's — so the picture does not jump when
    /// animation is switched off.
    #[frb(sync)]
    pub fn clear_mask_path_keys(&self, id: Uuid, time: BridgeRational) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut masks = layer.masks;
        let at = masks
            .iter()
            .position(|m| m.id == id)
            .ok_or(BridgeError::NoSuchMask)?;
        let time = Rational::new(time.num, time.den)
            .map_err(|_| BridgeError::InvalidKeyframes)?
            .checked_sub(offset)
            .map_err(|_| BridgeError::InvalidKeyframes)?;
        let mask = &mut masks[at];
        mask.path = mask.path_at(time.to_f64()).into_owned();
        mask.path_keys.clear();
        self.commit_masks(masks)
    }

    #[frb(ignore)]
    fn commit_masks(&self, masks: Vec<lumit_core::mask::Mask>) -> Result<(), BridgeError> {
        self.commit(lumit_core::Op::SetLayerMasks {
            comp: self.comp_id,
            layer: self.layer_id,
            masks,
        })
    }

    /// This layer's paint strokes, oldest first (K-227).
    #[frb(sync)]
    pub fn get_paint(&self) -> Result<Vec<BridgeStroke>, BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        Ok(layer
            .paint
            .iter()
            .map(|s| BridgeStroke::read_at(s, offset))
            .collect())
    }

    /// Add `stroke` on top of this layer's paint.
    ///
    /// The whole list is committed, because that is the op the engine has and
    /// it is exactly invertible (`SetLayerPaint`): one stroke is one undo step,
    /// which is what `Ctrl+Z` after a brush drag has to mean.
    ///
    /// A stroke with no points is refused — there is no gesture in it, and it
    /// would be a Timeline row with nothing behind it.
    #[frb(sync)]
    pub fn add_stroke(&self, stroke: BridgeStroke) -> Result<(), BridgeError> {
        if stroke.points.is_empty() {
            return Err(BridgeError::EmptyStroke);
        }
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut strokes = layer.paint;
        strokes.push(stroke.write_at(offset)?);
        self.commit_paint(strokes)
    }

    /// Replace one stroke — its path, its colour, its width, its name. Named by
    /// id, so a stale reference is a calm error rather than an edit landing on
    /// whichever stroke happens to sit at that index now.
    #[frb(sync)]
    pub fn set_stroke(&self, stroke: BridgeStroke) -> Result<(), BridgeError> {
        if stroke.points.is_empty() {
            return Err(BridgeError::EmptyStroke);
        }
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let mut strokes = layer.paint;
        let at = strokes
            .iter()
            .position(|s| s.id == stroke.id)
            .ok_or(BridgeError::NoSuchStroke)?;
        strokes[at] = stroke.write_at(offset)?;
        self.commit_paint(strokes)
    }

    /// Remove a stroke by id.
    #[frb(sync)]
    pub fn delete_stroke(&self, id: Uuid) -> Result<(), BridgeError> {
        let mut strokes = self.item()?.paint;
        let before = strokes.len();
        strokes.retain(|s| s.id != id);
        if strokes.len() == before {
            return Err(BridgeError::NoSuchStroke);
        }
        self.commit_paint(strokes)
    }

    /// Take the last stroke off — the undo inside the tool, for a brush drag
    /// that went wrong. Errors when there is nothing painted.
    #[frb(sync)]
    pub fn delete_last_stroke(&self) -> Result<(), BridgeError> {
        let mut strokes = self.item()?.paint;
        if strokes.pop().is_none() {
            return Err(BridgeError::NoSuchStroke);
        }
        self.commit_paint(strokes)
    }

    #[frb(ignore)]
    fn commit_paint(
        &self,
        strokes: Vec<lumit_core::paint::PaintStroke>,
    ) -> Result<(), BridgeError> {
        self.commit(lumit_core::Op::SetLayerPaint {
            comp: self.comp_id,
            layer: self.layer_id,
            strokes,
        })
    }

    /// This shape layer's contents, bottom of the stack first (K-237).
    ///
    /// Empty on a layer that is not a shape, rather than an error: the Timeline
    /// asks every row what it has to list, exactly as it asks about masks.
    #[frb(sync)]
    pub fn get_shape_contents(&self) -> Result<Vec<BridgeShapeItem>, BridgeError> {
        let lumit_core::model::LayerKind::Shape { contents } = self.item()?.kind else {
            return Ok(Vec::new());
        };
        let offset = self.item()?.start_offset.0;
        Ok(contents
            .iter()
            .map(|i| BridgeShapeItem::read_at(i, offset))
            .collect())
    }

    /// Replace this shape layer's whole contents.
    ///
    /// The whole list, exactly invertible (`SetShapeContents`), the same shape
    /// of edit as a mask list or a paint list: an add, a delete, a recolour and
    /// a path edit are one kind of thing and each is one undo step.
    ///
    /// A path of fewer than two vertices is refused, as a mask's is: it is not
    /// a shape, and it would be a Timeline row with nothing behind it.
    ///
    /// **The art that was not edited does not move** (K-308). A shape layer's
    /// picture is its art's bounding box, and the layer's origin is that box's
    /// top-left corner — so growing the box leftwards, which is what dragging
    /// the left-most point left does, would slide every *other* point right by
    /// the same amount. Position follows the corner to cancel that, in the same
    /// op, so one drag is still one undo step.
    ///
    /// **`at` is where the playhead is**, and it matters only for an item whose
    /// **shape** is keyed (K-606): the dragged vertices land on the key sitting
    /// there, or plant one holding them, exactly as they do on a mask (K-340).
    /// Pass `None` for an edit that is not a shape edit — an opacity drag, a
    /// rename, a colour — and the keys are carried through untouched.
    #[frb(sync)]
    pub fn set_shape_contents(
        &self,
        contents: Vec<BridgeShapeItem>,
        at: Option<BridgeRational>,
    ) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Shape { contents: before } = &layer.kind else {
            return Err(BridgeError::NotShape);
        };
        if contents.iter().any(|i| i.vertices.len() < 2) {
            return Err(BridgeError::EmptyPath);
        }
        let offset = layer.start_offset.0;
        let at = match at {
            Some(t) => {
                Some(Rational::new(t.num, t.den).map_err(|_| BridgeError::InvalidKeyframes)?)
            }
            None => None,
        };
        // An item the layer already had keeps what this type does not carry —
        // its shape keys and its `extra`; one that did not exist a moment ago
        // has neither to keep.
        let items: Vec<lumit_core::shape::ShapeItem> = contents
            .iter()
            .map(|i| match before.iter().find(|p| p.id == i.id) {
                Some(previous) => i.write_item_over(previous, offset, at),
                None => i.write_item(offset),
            })
            .collect::<Result<_, _>>()?;
        self.commit_shape_items(&layer, before, items)
    }

    /// The op (or the batch) that writes a shape layer's whole contents, with
    /// the layer's position following the art's corner (K-308).
    ///
    /// Shared by [`Self::set_shape_contents`] and the four shape-key edits, so
    /// keying, re-timing and un-keying a morphing path move the layer exactly
    /// as dragging one of its points does.
    #[frb(ignore)]
    fn commit_shape_items(
        &self,
        layer: &lumit_core::model::Layer,
        before: &[lumit_core::shape::ShapeItem],
        items: Vec<lumit_core::shape::ShapeItem>,
    ) -> Result<(), BridgeError> {
        // Both boxes on the same clock — the head of the layer — so the delta
        // is the edit's own, not the repeater's animation moving underneath it.
        let shift = match (
            lumit_core::shape::contents_bounds(before, 0.0),
            lumit_core::shape::contents_bounds(&items, 0.0),
        ) {
            (Some((x0, y0, _, _)), Some((x1, y1, _, _))) => (x1 - x0, y1 - y0),
            // Art appearing or going away entirely leaves the layer where it
            // is: there is no corner to follow.
            _ => (0.0, 0.0),
        };
        let mut ops = vec![lumit_core::Op::SetShapeContents {
            comp: self.comp_id,
            layer: self.layer_id,
            contents: items,
        }];
        // Only a still position can follow the corner: a keyframed one has no
        // single value to add to, and moving one key of a curve is an edit the
        // gesture never asked for.
        for (prop, delta, property) in [
            (
                lumit_core::model::TransformProp::PositionX,
                shift.0,
                &layer.transform.position_x,
            ),
            (
                lumit_core::model::TransformProp::PositionY,
                shift.1,
                &layer.transform.position_y,
            ),
        ] {
            if delta == 0.0 || property.is_animated() {
                continue;
            }
            let lumit_core::anim::Animation::Static(value) = &property.animation else {
                continue;
            };
            ops.push(lumit_core::Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop,
                animation: lumit_core::anim::Animation::Static(value + delta),
            });
        }
        let op = if ops.len() == 1 {
            ops.remove(0)
        } else {
            lumit_core::Op::Batch { ops }
        };
        self.commit(op)
    }

    /// Add one piece of art on top of this shape layer's stack.
    #[frb(sync)]
    pub fn add_shape_item(&self, item: BridgeShapeItem) -> Result<(), BridgeError> {
        let mut contents = self.get_shape_contents()?;
        let lumit_core::model::LayerKind::Shape { .. } = self.item()?.kind else {
            return Err(BridgeError::NotShape);
        };
        contents.push(item);
        self.set_shape_contents(contents, None)
    }

    /// Key this item's **shape** at `time`, or take the key already there away
    /// (K-606) — the diamond on a shape item's Path row, and the mask's own
    /// gesture (K-339, K-340) applied to the other thing in the document that
    /// holds a path.
    ///
    /// A planted key holds the shape the item is *already showing* at that
    /// moment, so pressing it never moves anything. `time` is composition time;
    /// the layer's own offset is taken back off inside.
    #[frb(sync)]
    pub fn toggle_shape_path_key(&self, id: Uuid, time: BridgeRational) -> Result<(), BridgeError> {
        self.edit_shape_item(id, time, |item, time| {
            if let Some(i) = item.path_keys.iter().position(|k| k.time == time) {
                item.path_keys.remove(i);
            } else {
                let path = item.path_at(time.to_f64()).into_owned();
                let at = item
                    .path_keys
                    .iter()
                    .position(|k| k.time > time)
                    .unwrap_or(item.path_keys.len());
                item.path_keys.insert(
                    at,
                    lumit_core::mask::PathKeyframe {
                        time,
                        path,
                        interp_in: lumit_core::anim::SideInterp::Linear,
                        interp_out: lumit_core::anim::SideInterp::Linear,
                    },
                );
            }
        })
    }

    /// Stop this item's shape animating, keeping the shape it shows at `time`
    /// (K-606) — the stopwatch turning off, and what it does everywhere else:
    /// the shape that stays is the one the playhead is over, so the picture
    /// does not jump.
    #[frb(sync)]
    pub fn clear_shape_path_keys(&self, id: Uuid, time: BridgeRational) -> Result<(), BridgeError> {
        self.edit_shape_item(id, time, |item, time| {
            item.path = item.path_at(time.to_f64()).into_owned();
            item.path_keys.clear();
        })
    }

    /// Drag one of this item's shape keys along the timeline (K-606) — the lane
    /// diamond, which moves a path key exactly as it moves a scalar's.
    ///
    /// Refused, with `false`, when the move would land on or step over a
    /// neighbour: keys are sorted with unique times and the evaluator walks them
    /// assuming so.
    #[frb(sync)]
    pub fn move_shape_path_key(
        &self,
        id: Uuid,
        from: BridgeRational,
        to: BridgeRational,
    ) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let lumit_core::model::LayerKind::Shape { contents: before } = &layer.kind else {
            return Err(BridgeError::NotShape);
        };
        let local = |t: BridgeRational| -> Result<Rational, BridgeError> {
            Rational::new(t.num, t.den)
                .map_err(|_| BridgeError::InvalidKeyframes)?
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)
        };
        let (from, to) = (local(from)?, local(to)?);
        let mut items = before.clone();
        let at = items
            .iter()
            .position(|i| i.id == id)
            .ok_or(BridgeError::InvalidItem)?;
        let item = &mut items[at];
        let Some(i) = item.path_keys.iter().position(|k| k.time == from) else {
            return Ok(false);
        };
        for (j, key) in item.path_keys.iter().enumerate() {
            if j == i {
                continue;
            }
            if (j < i && key.time >= to) || (j > i && key.time <= to) {
                return Ok(false);
            }
        }
        item.path_keys[i].time = to;
        self.commit_shape_items(&layer, before, items)?;
        Ok(true)
    }

    /// Re-time and re-ease this item's shape keys in one write (K-606) — what
    /// the graph editor commits when a handle is dragged, and what a lane drag
    /// of several keys at once needs.
    ///
    /// `keys` must name every key the item has, in order; their `value` is
    /// ignored, because a path key holds a shape rather than a number. Refused
    /// as a whole if the times are not strictly ascending.
    #[frb(sync)]
    pub fn set_shape_path_keys(
        &self,
        id: Uuid,
        keys: Vec<BridgeKeyframe>,
    ) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let lumit_core::model::LayerKind::Shape { contents: before } = &layer.kind else {
            return Err(BridgeError::NotShape);
        };
        let mut items = before.clone();
        let at = items
            .iter()
            .position(|i| i.id == id)
            .ok_or(BridgeError::InvalidItem)?;
        if keys.len() != items[at].path_keys.len() {
            return Ok(false);
        }
        let mut written = Vec::with_capacity(keys.len());
        for (key, existing) in keys.iter().zip(items[at].path_keys.iter()) {
            let time = Rational::new(key.time.num, key.time.den)
                .map_err(|_| BridgeError::InvalidKeyframes)?
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)?;
            if written
                .last()
                .is_some_and(|p: &lumit_core::mask::PathKeyframe| time <= p.time)
            {
                return Ok(false);
            }
            written.push(lumit_core::mask::PathKeyframe {
                time,
                path: existing.path.clone(),
                interp_in: key.interp_in.write(),
                interp_out: key.interp_out.write(),
            });
        }
        items[at].path_keys = written;
        self.commit_shape_items(&layer, before, items)?;
        Ok(true)
    }

    /// Read this layer's art, change the one item `id` names at layer-local
    /// `time`, and commit the list — the shape of every shape-key edit.
    #[frb(ignore)]
    fn edit_shape_item(
        &self,
        id: Uuid,
        time: BridgeRational,
        change: impl FnOnce(&mut lumit_core::shape::ShapeItem, Rational),
    ) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let lumit_core::model::LayerKind::Shape { contents: before } = &layer.kind else {
            return Err(BridgeError::NotShape);
        };
        let time = Rational::new(time.num, time.den)
            .map_err(|_| BridgeError::InvalidKeyframes)?
            .checked_sub(offset)
            .map_err(|_| BridgeError::InvalidKeyframes)?;
        let mut items = before.clone();
        let at = items
            .iter()
            .position(|i| i.id == id)
            .ok_or(BridgeError::InvalidItem)?;
        change(&mut items[at], time);
        self.commit_shape_items(&layer, before, items)
    }

    /// The clips on this Sequence layer, in the order it holds them.
    ///
    /// An empty list on a layer that is not a Sequence, rather than an error:
    /// the Timeline asks every row whether it has clips to draw, and a footage
    /// row simply has none.
    #[frb(sync)]
    pub fn get_clips(&self) -> Result<Vec<BridgeClip>, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Ok(Vec::new());
        };
        let proj = self.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Composition(comp)) = doc.item(self.comp_id) else {
            return Err(BridgeError::InvalidItem);
        };
        Ok(clips
            .iter()
            .map(|c| {
                bridge_clip(
                    comp,
                    &layer,
                    c,
                    clip_source_duration(&state, &doc, c.source),
                )
            })
            .collect())
    }

    /// Set one clip's playback speed, as a percentage (K-247, K-248).
    ///
    /// The clip keeps its place on the row — its start and its length are
    /// untouched, so an edit point already on a beat stays on it (K-022) —
    /// and the stretch of source it plays follows from the speed. Its first
    /// frame is pinned, so re-speeding never moves where a clip begins
    /// (K-070).
    ///
    /// `end_percent` makes it a ramp, running straight from one speed to the
    /// other; leave it equal to `percent` for a constant speed. Negative runs
    /// the clip backwards.
    #[frb(sync)]
    pub fn set_clip_speed(
        &self,
        clip: Uuid,
        percent: f64,
        end_percent: f64,
    ) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let index = clips
            .iter()
            .position(|c| c.id == clip)
            .ok_or(BridgeError::InvalidLayer)?;
        let rate = |v: f64| {
            lumit_core::time::Rational::from_f64_on_grid(
                v / 100.0,
                lumit_core::time::Rational::FLICK_DEN,
            )
            .map_err(|_| BridgeError::InvalidTime)
        };
        let mut clips = clips.clone();
        clips[index] = clips[index].with_ramp(rate(percent)?, rate(end_percent)?);
        self.commit_clips(clips)
    }

    /// Replace one clip's whole retime map, keyed in clip-local time.
    ///
    /// The envelope in the sequence view writes through here: it speaks the
    /// same keyframes the graph editor's Vegas lens does (K-249 made them one
    /// representation), so one editor serves both. The clip keeps its place;
    /// what it plays follows from the map.
    #[frb(sync)]
    pub fn set_clip_retime(&self, clip: Uuid, value: BridgeScalar) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let index = clips
            .iter()
            .position(|c| c.id == clip)
            .ok_or(BridgeError::InvalidLayer)?;
        let mut clips = clips.clone();
        // Clip time, so no layer offset is applied on the way in.
        let map = lumit_core::anim::Property {
            animation: value.animation_at(lumit_core::time::Rational::ZERO)?,
            extra: serde_json::Map::new(),
        };
        // What the clip *asks* of its source follows from the map's last key.
        if let Some(end) = map_end_value(&map) {
            clips[index].source_out = end;
        }
        clips[index].retime = Some(map);
        self.commit_clips(clips)
    }

    /// Slide a clip along the row so it starts at `to_frame` (docs/04 §8.2).
    ///
    /// Its length, its trim and its map are untouched — the same frames play,
    /// just earlier or later. Refused where it would start before the layer's
    /// own zero.
    #[frb(sync)]
    pub fn slide_clip(&self, clip: Uuid, to_frame: i64) -> Result<(), BridgeError> {
        let (mut clips, index) = self.clips_and_index(clip)?;
        let comp = self.composition()?;
        let layer = self.item()?;

        // The travel, as a signed time: `to_frame` may be before the start of
        // the composition, and a frame count is the only place the sign
        // survives cleanly.
        let start_frame = comp
            .frame_rate
            .frame_at(lumit_core::time::CompTime(clips[index].place_start));
        let moved = to_frame - start_frame;
        let step = comp
            .frame_rate
            .time_of_frame(moved.unsigned_abs() as i64)
            .map_err(|_| BridgeError::InvalidTime)?
            .0;
        let zero = Rational::ZERO;
        let delta = if moved < 0 {
            zero.checked_sub(step)
                .map_err(|_| BridgeError::InvalidTime)?
        } else {
            step
        };

        let wanted = clips[index]
            .place_start
            .checked_add(delta)
            .map_err(|_| BridgeError::InvalidTime)?;

        // **Before the layer's own zero, the layer moves.** A clip's place is
        // layer time and cannot go negative, so dragging one back past the
        // start carries the whole layer earlier — exactly what dragging any
        // other layer's bar before the start of the composition does. Every
        // *other* clip is pushed the same amount later in layer time, so it
        // stays where it was on the comp's clock and only the dragged one
        // actually moves.
        if wanted.is_negative() {
            let shift = Rational::ZERO
                .checked_sub(wanted)
                .map_err(|_| BridgeError::InvalidTime)?;
            for c in clips.iter_mut() {
                c.place_start = c
                    .place_start
                    .checked_add(shift)
                    .map_err(|_| BridgeError::InvalidTime)?;
            }
            clips[index].place_start = Rational::ZERO;
            let dropped = clips[index].id;
            let clips = lumit_core::sequence::overwrite_with(&clips, dropped);
            let offset = layer
                .start_offset
                .0
                .checked_sub(shift)
                .map_err(|_| BridgeError::InvalidTime)?;
            return self.commit_clips_with_offset(clips, lumit_core::time::CompTime(offset));
        }

        clips[index] = clips[index].slide(delta).ok_or(BridgeError::InvalidTime)?;
        let dropped = clips[index].id;
        self.commit_clips(lumit_core::sequence::overwrite_with(&clips, dropped))
    }

    /// Trim one edge of a clip inward (docs/04 §8.2, non-ripple).
    ///
    /// `start_frame` and `end_frame` are where the clip's edges should land.
    /// An edge moving **inward** crops the map there; one moving **outward**
    /// carries it on at the speed it was already going (§7.3), which is what
    /// lets a clip be lengthened again after a cut. Running past the media it
    /// has is legal — that is overrun, and it renders as a held frame — so it
    /// is not refused. Nothing else on the row moves: no ripple, ever (K-022).
    #[frb(sync)]
    pub fn trim_clip(
        &self,
        clip: Uuid,
        start_frame: i64,
        end_frame: i64,
    ) -> Result<(), BridgeError> {
        let (mut clips, index) = self.clips_and_index(clip)?;
        let comp = self.composition()?;
        let at = |f: i64| {
            comp.frame_rate
                .time_of_frame(f.max(0))
                .map(|t| t.0)
                .map_err(|_| BridgeError::InvalidTime)
        };
        let (start, end) = (at(start_frame)?, at(end_frame)?);
        let mut next = clips[index].clone();
        if end < next.place_end() {
            next = next.trim_end(end).ok_or(BridgeError::InvalidTime)?;
        } else if end > next.place_end() {
            next = next.extend_end(end).ok_or(BridgeError::InvalidTime)?;
        }
        if start > next.place_start {
            next = next.trim_start(start).ok_or(BridgeError::InvalidTime)?;
        } else if start < next.place_start {
            next = next.extend_start(start).ok_or(BridgeError::InvalidTime)?;
        }
        clips[index] = next;
        self.commit_clips(clips)
    }

    /// This layer's clips and the index of `clip` among them.
    #[frb(ignore)]
    fn clips_and_index(
        &self,
        clip: Uuid,
    ) -> Result<(Vec<lumit_core::sequence::Clip>, usize), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let index = clips
            .iter()
            .position(|c| c.id == clip)
            .ok_or(BridgeError::InvalidLayer)?;
        Ok((clips.clone(), index))
    }

    /// A thumbnail of the **first frame this clip shows** (K-248).
    ///
    /// Not the file's first frame: a clip after a cut starts part way in, and
    /// a row of thumbnails that all showed frame zero would say nothing about
    /// which clip is which. The moment is read through the clip's own map, so
    /// a re-speeded clip still shows the frame it actually opens on.
    ///
    /// `None` when the media will not open, when the source is a comp rather
    /// than footage (there is nothing on disk to decode), or in a build with
    /// no media feature. Decoded once per (item, size, frame) and cached.
    pub fn clip_thumbnail(
        &self,
        clip: Uuid,
        max_edge: u32,
    ) -> Result<Option<crate::api::state::BridgeRenderedFrame>, BridgeError> {
        let (clips, index) = self.clips_and_index(clip)?;
        let clip = &clips[index];
        let lumit_core::sequence::ClipSource::Footage(item) = clip.source else {
            return Ok(None);
        };
        // The clip's own opening source moment, through whatever map it plays
        // by — which is the identity when it has none of its own.
        let opens_at = clip.source_time(clip.place_start.to_f64());

        #[cfg(feature = "media")]
        {
            let project = self.project()?;
            // **Everything under the read lock, then let it go.** Decoding a
            // video frame is slow enough that holding the project across it
            // stalls every other reader, and the render worker is one of them
            // (docs/14 §3). The lock comes back only to store the result.
            let (src, at, cached) = {
                let proj = project.read().map_err(|_| BridgeError::ReadFailed)?;
                let doc = proj.store.snapshot();
                let Some(lumit_core::model::ProjectItem::Footage(f)) = doc.item(item) else {
                    return Ok(None);
                };
                let Some(src) = crate::api::footage::FootageReference::resolve_source(&proj, f)
                else {
                    return Ok(None);
                };
                // The media's own rate turns its seconds into its frames.
                let fps = crate::probe::ensure_probed(&src)
                    .and_then(|i| i.video.as_ref().map(|v| v.fps()))
                    .filter(|fps| *fps > 0.0)
                    .unwrap_or(1.0);
                let at = (opens_at * fps).round() as i64;
                let cached = crate::media::thumb_cached(&proj.media, item, max_edge, at);
                (src, at, cached)
            };

            let thumb = match cached {
                Some(hit) => hit,
                None => {
                    let Some(decoded) = crate::media::thumb_decode(&src, max_edge, at) else {
                        return Ok(None);
                    };
                    if let Ok(mut proj) = project.write() {
                        crate::media::thumb_store(&mut proj.media, item, max_edge, at, &decoded);
                    }
                    decoded
                }
            };
            let (width, height, rgba) = thumb;
            Ok(Some(crate::api::state::BridgeRenderedFrame {
                frame: at.unsigned_abs(),
                width,
                height,
                rgba,
            }))
        }

        #[cfg(not(feature = "media"))]
        {
            let _ = (item, opens_at, max_edge);
            Ok(None)
        }
    }

    /// Turn a Sequence layer back into a plain Footage layer (K-248).
    ///
    /// The way out of the clip-editing surface, and it must exist: converting
    /// in is offered to anyone, so a user who tries it has to be able to
    /// change their mind.
    ///
    /// **It keeps the first clip's source and its trim, and nothing else.**
    /// The cuts, the gaps and the per-clip ramps have no home on a layer that
    /// holds one uncut piece of footage, and inventing somewhere to put them
    /// would be worse than saying plainly that they go. A row of several clips
    /// is refused outright rather than silently losing all but one: the user
    /// can delete the clips they do not want first, which is a decision only
    /// they can make.
    #[frb(sync)]
    pub fn convert_from_sequenced(&self) -> Result<(), BridgeError> {
        use lumit_core::model::LayerKind;

        let layer = self.item()?;
        let LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let [clip] = clips.as_slice() else {
            return Err(BridgeError::ManyClips);
        };
        let lumit_core::sequence::ClipSource::Footage(item) = clip.source else {
            return Err(BridgeError::NotFootage);
        };
        let comp = self.composition()?;
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;

        let mut converted = layer.clone();
        converted.kind = LayerKind::Footage { item };
        converted.interpolation = clip.interpolation.clone();
        // The clip's own map becomes the layer's, which is exact: the clip
        // spans the whole layer, so clip time and layer time are the same
        // clock (K-249 made them the same kind of map, so nothing converts).
        converted.retime = clip.retime.clone();

        self.commit(lumit_core::Op::Batch {
            ops: vec![
                lumit_core::Op::RemoveLayer {
                    comp: self.comp_id,
                    layer: self.layer_id,
                },
                lumit_core::Op::AddLayer {
                    comp: self.comp_id,
                    index,
                    layer: Box::new(converted),
                },
            ],
        })
    }

    /// The shape of this Sequence layer — where its cuts fall and how each
    /// piece is ramped — as text, for [`Self::paste_sequence_shape`] (K-248).
    ///
    /// `clip` reads that clip alone; `None` reads the whole row. What comes
    /// back carries no *source*: applying it keeps the target's own media,
    /// which is the point — cutting a depth pass to the same beats as the
    /// footage it belongs to is work nobody should do twice by hand, and by
    /// eye the two always drift.
    #[frb(sync)]
    pub fn copy_sequence_shape(&self, clip: Option<Uuid>) -> Result<String, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let taken: Vec<_> = match clip {
            Some(id) => clips.iter().filter(|c| c.id == id).cloned().collect(),
            None => clips.clone(),
        };
        serde_json::to_string(&lumit_core::sequence::SequenceShape::of(&taken))
            .map_err(|_| BridgeError::InvalidItem)
    }

    /// Cut and ramp this Sequence layer to the shape in `text`, keeping its
    /// own media (K-248).
    ///
    /// The row keeps the source its first clip already plays, and is rebuilt
    /// with the pieces the shape describes. A shape longer than this row
    /// reaches is applied as far as it goes: the piece straddling the end is
    /// trimmed to it and anything wholly beyond is dropped, so a shape taken
    /// from long footage lands sensibly on short footage rather than inventing
    /// a row that runs past its media.
    #[frb(sync)]
    pub fn paste_sequence_shape(&self, text: String) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let shape: lumit_core::sequence::SequenceShape =
            serde_json::from_str(&text).map_err(|_| BridgeError::InvalidItem)?;
        // The media this row plays, and how far it reaches — both taken from
        // the row as it stands, because neither is the shape's business.
        let source = clips.first().ok_or(BridgeError::NoClipThere)?.source;
        let limit = lumit_core::sequence::clips_span(clips)
            .map(|(_, end)| end)
            .ok_or(BridgeError::NoClipThere)?;
        let next = shape.apply(source, limit);
        if next.is_empty() {
            return Err(BridgeError::NoClipThere);
        }
        self.commit_clips(next)
    }

    /// Take one clip off the row, by id.
    ///
    /// What it leaves is a **gap**, not a closed row: nothing after it moves,
    /// so every edit point still standing keeps the beat it was cut to
    /// (K-022). `delete_clip_at` is the same thing aimed with the playhead;
    /// this is the one the sequence view's own menu uses, because there the
    /// clip has already been pointed at.
    #[frb(sync)]
    pub fn delete_clip(&self, clip: Uuid) -> Result<(), BridgeError> {
        let (mut clips, index) = self.clips_and_index(clip)?;
        clips.remove(index);
        self.commit_clips(clips)
    }

    /// Razor: cut the clip under `frame` in two, at the playhead.
    ///
    /// The two halves keep their places — a cut must not shift what comes after
    /// it, which is the beat-sync covenant (K-071). An **eased speed ramp cuts
    /// like anything else** (K-573): the map's cubic is split at the cut, so
    /// the two halves concatenate to the speed curve that was there before.
    /// What still refuses is a moment on one of the clip's own ends, and a
    /// retime driven by an expression — both calm errors rather than a cut that
    /// changes what plays.
    #[frb(sync)]
    pub fn cut_clip_at(&self, frame: i64) -> Result<(), BridgeError> {
        let (mut clips, index, tau) = self.clip_under(frame)?;
        let (left, right) = clips[index].cut(tau).ok_or(BridgeError::UncuttableClip)?;
        clips.splice(index..=index, [left, right]);
        self.commit_clips(clips)
    }

    /// Razor: split this layer in two at `frame` (docs/07 §4.4).
    ///
    /// After Effects' split, not a clip cut: the layer keeps everything it has —
    /// its source, effects, masks, parent, label and keyframes — and the copy
    /// takes the tail. Both halves keep the **same `start_offset`**, which is
    /// what makes the cut invisible: layer time is measured from that offset
    /// (K-213), so each half shows exactly the frames it showed before and every
    /// keyframe stays where it was on the comp's clock.
    ///
    /// One `Batch`, so it is one undo step — docs/07 §4.7 requires that of every
    /// destructive-feeling action, and a razor that took two would be two.
    ///
    /// The copy goes directly above the original, where a duplicate goes.
    /// `frame` must land strictly inside the layer's span: cutting at either end
    /// would make a layer of no length, so it is a calm error rather than a
    /// zero-length layer nobody asked for.
    #[frb(sync)]
    pub fn split_at(&self, frame: i64) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        let layer = self.item()?;
        let t = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        if t.0 <= layer.in_point.0 || t.0 >= layer.out_point.0 {
            return Err(BridgeError::NothingToSplit);
        }
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;

        // A retimed layer gets a keyframe *at the cut*, on both halves (K-221).
        //
        // Both halves keep the whole map, so without this the two speed ramps
        // stay welded together: editing one half's speed would bend the other
        // half's curve, because they are the same curve. A key at the cut gives
        // each half an end of its own to hold. It is inserted preserving the
        // shape, so the cut itself changes nothing that plays — and it goes in
        // *before* the clone, which is what puts it on both halves.
        // **Only a layer that has actually been retimed** (K-236). Switching
        // Retime on installs the identity map, and a map nobody has shaped
        // needs no key at the cut: both halves play their source at their own
        // clock whatever happens to the other. Keys appearing on a layer the
        // user never retimed are keys they then have to notice and remove.
        let retimed = layer
            .retime
            .as_ref()
            .is_some_and(|r| !lumit_core::model::Layer::is_identity_retime(r));
        let mut head = layer.clone();
        if let Some(retime) = head.retime.as_mut().filter(|_| retimed) {
            // Layer time, not comp time: keyframes live in the layer's own
            // clock, measured from its start offset (K-213).
            // A subtraction that cannot overflow is still a subtraction that
            // can: an unrepresentable time leaves the map alone rather than
            // taking the cut down with it.
            if let Ok(local) = t.0.checked_sub(head.start_offset.0) {
                retime.insert_key_preserving_shape(local);
            }
        }

        let mut tail = head.clone();
        tail.id = Uuid::now_v7();
        for effect in &mut tail.effects {
            effect.id = Uuid::now_v7();
        }
        tail.in_point = t;

        let mut ops = vec![lumit_core::Op::SetLayerSpan {
            comp: self.comp_id,
            layer: self.layer_id,
            in_point: layer.in_point,
            out_point: t,
            // Untouched: the offset is what makes both halves show the frames
            // they showed before the cut.
            start_offset: layer.start_offset,
        }];
        // The head keeps its id, so its new map is written to it by name; the
        // tail carries its copy of the map in the layer being added.
        if head.retime != layer.retime {
            ops.push(lumit_core::Op::SetRetimeProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                retime: head.retime.clone(),
            });
        }
        ops.push(lumit_core::Op::AddLayer {
            comp: self.comp_id,
            index,
            layer: Box::new(tail),
        });
        self.commit(lumit_core::Op::Batch { ops })
    }

    /// Delete the clip under `frame`, leaving a gap.
    ///
    /// A gap is legal on the Vegas surface (K-071), so the clips after it stay
    /// where they are rather than rippling back — again so a cut never moves
    /// anything that was already in time with the music.
    #[frb(sync)]
    pub fn delete_clip_at(&self, frame: i64) -> Result<(), BridgeError> {
        let (mut clips, index, _) = self.clip_under(frame)?;
        clips.remove(index);
        self.commit_clips(clips)
    }

    /// Turn a Footage layer into a Sequence layer holding one clip of the whole
    /// source — the way into the clip-editing surface.
    ///
    /// Remove-then-add at the same index rather than an in-place kind change,
    /// because a layer's kind is not something any single op edits; the batch
    /// makes it one undo step. Only footage converts.
    #[frb(sync)]
    pub fn convert_to_sequenced(&self) -> Result<(), BridgeError> {
        use lumit_core::model::LayerKind;
        use lumit_core::sequence::{Clip, ClipSource};
        use lumit_core::time::Rational;

        let layer = self.item()?;
        let LayerKind::Footage { item } = &layer.kind else {
            return Err(BridgeError::NotFootage);
        };
        let comp = self.composition()?;
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;

        // The layer's own span length is the fallback when the media has not
        // probed; a quarter-second floor keeps a clip from being unclickable.
        let span = (layer.out_point.0.to_f64() - layer.in_point.0.to_f64()).max(0.04);
        let duration =
            Rational::from_f64_on_grid(span, Rational::FLICK_DEN).unwrap_or(layer.out_point.0);

        let mut converted = layer.clone();
        converted.kind = LayerKind::Sequence {
            clips: vec![Clip {
                id: Uuid::now_v7(),
                source: ClipSource::Footage(*item),
                source_in: Rational::ZERO,
                source_out: duration,
                place_start: Rational::ZERO,
                place_duration: duration,
                // **The layer's own map comes with it.** A layer's Retime is
                // keyed in layer time and a clip's in clip time, and here they
                // are the same clock: the clip spans the whole layer, starting
                // at its zero. K-249 made the two the same kind of map, so
                // nothing is converted — it is the same keyframes, read
                // against the same instant. (They stop coinciding the moment
                // the clip is cut or slid, but by then the map is the clip's
                // and travels with it.)
                //
                // The mirror of `convert_from_sequenced`, which brings it
                // back the same way: converting one direction and back must
                // leave the layer playing what it played.
                retime: layer.retime.clone(),
                interpolation: layer.interpolation.clone(),
                extra: serde_json::Map::new(),
            }],
        };

        self.commit(lumit_core::Op::Batch {
            ops: vec![
                lumit_core::Op::RemoveLayer {
                    comp: self.comp_id,
                    layer: self.layer_id,
                },
                lumit_core::Op::AddLayer {
                    comp: self.comp_id,
                    index,
                    layer: Box::new(converted),
                },
            ],
        })
    }

    /// The adjustment switch (K-537): set this layer's own picture aside and
    /// run its effect stack on the composite beneath it, or give it back.
    ///
    /// **On every layer that shows something in the Viewer** — footage, solid,
    /// precomp, text, shape, sequence — and refused calmly
    /// ([`BridgeError::NotConvertible`]) on the four with no picture to set
    /// aside: a Camera, a Light, a Null and an Audio layer. A layer whose own
    /// visibility switch is off still takes it: what a layer *is* and whether
    /// it is being shown are two answers. Asking for the state the layer is
    /// already in writes nothing at all, so a redundant click leaves no undo
    /// step to walk back through.
    ///
    /// **Nothing is lost while it is on**, which is the whole reason this is a
    /// flag and not the kind flip K-484 built: the source, the masks and the
    /// transform stay put, so switching back is the layer exactly as it was.
    ///
    /// The one asymmetry is a layer *born* an adjustment
    /// ([`lumit_core::model::LayerKind::Adjustment`], what **New adjustment
    /// layer** makes): it has no picture to give back, so turning the switch
    /// off hands it a fresh comp-sized white solid — the asset **New solid**
    /// makes, from the helper both share — and normalises it to a solid with
    /// the flag off, all in one batch, which is one undo step.
    #[frb(sync)]
    pub fn set_adjustment(&self, on: bool) -> Result<(), BridgeError> {
        use lumit_core::model::LayerKind;

        let layer = self.item()?;
        if !layer.can_adjust() {
            return Err(BridgeError::NotConvertible);
        }
        if layer.is_adjustment() == on {
            return Ok(());
        }
        let (comp_id, layer_id) = (self.comp_id, self.layer_id);
        let off = lumit_core::Op::SetLayerAdjustment {
            comp: comp_id,
            layer: layer_id,
            adjustment: on,
        };
        if on || !matches!(layer.kind, LayerKind::Adjustment) {
            return self.commit(off);
        }
        // Born an adjustment and being switched off: it needs a picture.
        let comp = self.composition()?;
        let (def, _, mut ops) = {
            let proj = self.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            crate::edits::white_solid_ops(&state.store.snapshot(), comp.width, comp.height)
        };
        ops.push(lumit_core::Op::SetLayerKind {
            comp: comp_id,
            layer: layer_id,
            kind: Box::new(LayerKind::Solid { def }),
        });
        // Clears the flag too, so the layer leaves this batch as an ordinary
        // solid however it came in — one shape of "not an adjustment", not two.
        ops.push(off);
        self.commit(lumit_core::Op::Batch { ops })
    }

    /// The clips, the index of the one under `frame`, and the layer-local time
    /// there.
    #[frb(ignore)]
    fn clip_under(
        &self,
        frame: i64,
    ) -> Result<
        (
            Vec<lumit_core::sequence::Clip>,
            usize,
            lumit_core::time::Rational,
        ),
        BridgeError,
    > {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let comp = self.composition()?;
        let at = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        // Layer-local: the playhead less where this layer's own time 0 sits.
        let tau =
            at.0.checked_sub(layer.start_offset.0)
                .map_err(|_| BridgeError::InvalidTime)?;
        let index = clips
            .iter()
            .position(|c| c.contains(tau.to_f64()))
            .ok_or(BridgeError::NoClipThere)?;
        Ok((clips.clone(), index, tau))
    }

    #[frb(ignore)]
    /// Write a Sequence layer's clips, and bring its bar with them.
    ///
    /// **A Sequence layer's length is its clips' length** (K-248): first
    /// clip's start to last clip's end, so deleting an outermost clip or
    /// dragging one further out moves the end of the bar with it. Interior
    /// gaps stay gaps — they render transparent and are never closed (K-022).
    ///
    /// Batched with the clips rather than folded into `SetSequenceClips`,
    /// because the op's inverse is "the clips as they were" and a span quietly
    /// changed inside it would not come back on undo. A batch inverts
    /// member-wise, so both halves undo together for free.
    fn commit_clips(&self, clips: Vec<lumit_core::sequence::Clip>) -> Result<(), BridgeError> {
        let offset = self.item()?.start_offset;
        self.commit_clips_with_offset(clips, offset)
    }

    /// [`Self::commit_clips`], with the layer's own zero moving too — what a
    /// clip dragged back past the start of the row needs, since a clip's place
    /// is layer time and cannot go negative.
    #[frb(ignore)]
    fn commit_clips_with_offset(
        &self,
        clips: Vec<lumit_core::sequence::Clip>,
        start_offset: lumit_core::time::CompTime,
    ) -> Result<(), BridgeError> {
        let mut layer = self.item()?;
        layer.start_offset = start_offset;
        let set_clips = lumit_core::Op::SetSequenceClips {
            comp: self.comp_id,
            layer: self.layer_id,
            clips: clips.clone(),
        };
        // Clip places are in layer time; a span is in comp time, and the two
        // differ by the layer's own zero.
        let Some((start, end)) = lumit_core::sequence::clips_span(&clips) else {
            return self.commit(set_clips);
        };
        let offset = layer.start_offset.0;
        let (Ok(in_point), Ok(out_point)) = (offset.checked_add(start), offset.checked_add(end))
        else {
            return self.commit(set_clips);
        };
        if in_point == layer.in_point.0 && out_point == layer.out_point.0 {
            return self.commit(set_clips);
        }
        self.commit(lumit_core::Op::Batch {
            ops: vec![
                set_clips,
                lumit_core::Op::SetLayerSpan {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    in_point: lumit_core::time::CompTime(in_point),
                    out_point: lumit_core::time::CompTime(out_point),
                    start_offset,
                },
            ],
        })
    }

    /// The project item this layer draws from, when it has one.
    ///
    /// `None` for the kinds that have no source of their own — a solid's
    /// definition, an adjustment layer, a camera, a text layer. The Viewer needs
    /// it to ask whether a footage layer's file is still there, which is what
    /// puts the missing-media slate on screen instead of a black frame.
    #[frb(sync)]
    pub fn get_source_item(&self) -> Result<Option<ItemReference>, BridgeError> {
        use lumit_core::model::LayerKind;
        let layer = self.item()?;
        let id = match layer.kind {
            LayerKind::Footage { item, .. } => item,
            LayerKind::Precomp { comp } => comp,
            LayerKind::Solid { def } => def,
            LayerKind::Text { .. }
            | LayerKind::Shape { .. }
            | LayerKind::Camera { .. }
            | LayerKind::Light { .. }
            | LayerKind::Sequence { .. }
            | LayerKind::Adjustment
            | LayerKind::Null => return Ok(None),
        };

        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = proj.store.snapshot();
        Ok(doc
            .item(id)
            .map(|item| crate::api::project_item::item_reference(self.project_id, item)))
    }

    /// The frame to open this layer's nested composition on, entering it from
    /// `outer_frame` on this comp's ruler (K-624).
    ///
    /// `None` when the layer is not a Precomp layer, or when the comp it names
    /// has gone — the caller then opens wherever it was going to anyway.
    ///
    /// Here rather than in Dart because the answer is the engine's own: it runs
    /// through the layer's start offset and Retime map, and the two comps may
    /// keep different frame rates, so an outer frame and an inner frame are not
    /// the same count of anything (docs/14 §2).
    #[frb(sync)]
    pub fn nested_entry_frame(&self, outer_frame: i64) -> Result<Option<i64>, BridgeError> {
        use lumit_core::model::LayerKind;
        use lumit_core::time::{CompTime, Rational};

        let layer = self.item()?;
        let LayerKind::Precomp { comp: nested } = layer.kind else {
            return Ok(None);
        };
        let outer = self.composition()?;

        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = proj.store.snapshot();
        let Some(inner) = doc.comp(nested) else {
            return Ok(None);
        };

        let t = outer
            .frame_rate
            .time_of_frame(outer_frame)
            .map_err(|_| BridgeError::InvalidComp)?;
        let entry = layer.entry_time(t.0.to_f64(), inner.duration.0.to_f64());
        let grid = Rational::from_f64_on_grid(entry, Rational::FLICK_DEN)
            .map_err(|_| BridgeError::InvalidComp)?;
        // `duration_frames` counts one past the last frame the transport can
        // reach, so the end of the nested comp is the frame before it.
        let last = inner
            .frame_rate
            .frame_at(CompTime(inner.duration.0))
            .saturating_sub(1)
            .max(0);
        Ok(Some(
            inner.frame_rate.frame_at(CompTime(grid)).clamp(0, last),
        ))
    }

    /// What kind of source this layer has.
    #[frb(sync)]
    pub fn get_kind(&self) -> Result<BridgeLayerKind, BridgeError> {
        Ok(bridge_kind(&self.item()?))
    }

    /// All the switches at once.
    #[frb(sync)]
    pub fn get_switches(&self) -> Result<BridgeLayerSwitches, BridgeError> {
        Ok(bridge_switches(&self.item()?))
    }

    /// Set one switch. One op each, so each click is one undo step.
    #[frb(sync)]
    pub fn set_switch(&self, switch: BridgeLayerSwitch, on: bool) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(match switch {
            BridgeLayerSwitch::Visible => lumit_core::Op::SetLayerVisible {
                comp,
                layer,
                visible: on,
            },
            BridgeLayerSwitch::Audible => lumit_core::Op::SetLayerAudible {
                comp,
                layer,
                audible: on,
            },
            BridgeLayerSwitch::Locked => lumit_core::Op::SetLayerLocked {
                comp,
                layer,
                locked: on,
            },
            BridgeLayerSwitch::Solo => lumit_core::Op::SetLayerSolo {
                comp,
                layer,
                solo: on,
            },
            BridgeLayerSwitch::ThreeD => lumit_core::Op::SetLayerThreeD {
                comp,
                layer,
                three_d: on,
            },
            BridgeLayerSwitch::Fx => lumit_core::Op::SetLayerFx {
                comp,
                layer,
                fx: on,
            },
            BridgeLayerSwitch::MotionBlur => lumit_core::Op::SetLayerMotionBlur {
                comp,
                layer,
                motion_blur: on,
            },
            BridgeLayerSwitch::Collapse => lumit_core::Op::SetLayerCollapse {
                comp,
                layer,
                collapse: on,
            },
            BridgeLayerSwitch::Shy => lumit_core::Op::SetLayerShy {
                comp,
                layer,
                shy: on,
            },
            BridgeLayerSwitch::AcceptsLights => lumit_core::Op::SetLayerAcceptsLights {
                comp,
                layer,
                accepts_lights: on,
            },
            BridgeLayerSwitch::Guide => lumit_core::Op::SetLayerGuide {
                comp,
                layer,
                guide: on,
            },
            // The one switch whose write is not always a single op: turning it
            // off on a layer born an adjustment has to give it a picture again
            // (K-537). Delegated rather than copied, so the Timeline's plural
            // switch handler and [`Self::set_adjustment`] cannot come to
            // different answers.
            BridgeLayerSwitch::Adjustment => return self.set_adjustment(on),
        })
    }

    /// The label-colour index: which chip the Timeline draws beside the layer
    /// number, as an index into the theme's label palette (TL2).
    #[frb(sync)]
    pub fn get_label(&self) -> Result<u8, BridgeError> {
        Ok(self.item()?.label)
    }

    #[frb(sync)]
    pub fn set_label(&self, label: u8) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerLabel { comp, layer, label })
    }

    /// This layer's own markers (docs/03 §11), drawn on its bar rather than on
    /// the comp's ruler.
    ///
    /// The layer's, not the source composition's: a comp dropped into another
    /// brings a **copy** along, and from then on the two lists are unrelated
    /// (K-254). Deleting one here never reaches into another composition.
    #[frb(sync)]
    pub fn get_markers(&self) -> Result<Vec<BridgeMarker>, BridgeError> {
        let rate = self.composition()?.frame_rate;
        Ok(self
            .item()?
            .markers
            .iter()
            .map(|m| bridge_marker(m, rate))
            .collect())
    }

    /// Replace this layer's whole marker list — one op, trivially invertible,
    /// the same shape as the composition's.
    #[frb(sync)]
    pub fn set_markers(&self, markers: Vec<BridgeMarker>) -> Result<(), BridgeError> {
        // Merged onto the layer's current list, so a marker's kind, duration
        // and unknown fields survive a drag or a rename (K-270).
        let markers = core_markers(
            markers,
            &self.item()?.markers,
            self.composition()?.frame_rate,
        )?;
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerMarkers {
            comp,
            layer,
            markers,
        })
    }

    /// Where this layer sits on the comp timeline.
    #[frb(sync)]
    pub fn get_span(&self) -> Result<BridgeSpan, BridgeError> {
        let layer = self.item()?;
        Ok(BridgeSpan {
            in_point: rational_of(layer.in_point.0),
            out_point: rational_of(layer.out_point.0),
            start_offset: rational_of(layer.start_offset.0),
        })
    }

    /// Move or trim the layer. One op, so a drag that changes the in point and
    /// the start offset together — a slip edit — is still one undo step.
    ///
    /// An out point at or before the in point is refused by the op rather than
    /// clamped here: a zero-length layer is not something the Timeline should be
    /// able to produce by accident, and silently widening it would hide the bug
    /// that produced it.
    #[frb(sync)]
    pub fn set_span(&self, span: BridgeSpan) -> Result<(), BridgeError> {
        use lumit_core::time::CompTime;
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerSpan {
            comp,
            layer,
            in_point: CompTime(comp_time(span.in_point)?),
            out_point: CompTime(comp_time(span.out_point)?),
            start_offset: CompTime(comp_time(span.start_offset)?),
        })
    }

    /// This layer's blend mode, as an index into [`list_blend_modes`].
    #[frb(sync)]
    pub fn get_blend(&self) -> Result<u32, BridgeError> {
        let blend = self.item()?.blend;
        Ok(lumit_core::model::BlendMode::ALL
            .iter()
            .position(|b| *b == blend)
            .unwrap_or(0) as u32)
    }

    #[frb(sync)]
    pub fn set_blend(&self, index: u32) -> Result<(), BridgeError> {
        let blend = *lumit_core::model::BlendMode::ALL
            .get(index as usize)
            .ok_or(BridgeError::InvalidBlendMode)?;
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerBlend { comp, layer, blend })
    }

    /// The layer used as this one's matte, if any.
    #[frb(sync)]
    pub fn get_matte(&self) -> Result<Option<BridgeMatte>, BridgeError> {
        use lumit_core::model::MatteChannel;
        Ok(self.item()?.matte.map(|m| BridgeMatte {
            layer: m.layer,
            luma: matches!(m.channel, MatteChannel::Luma),
            inverted: m.inverted,
        }))
    }

    /// Point this layer at another as its matte, or clear it with `None`.
    ///
    /// A matte naming a layer that is not there degrades to "no matte" at render
    /// (docs/03 §5.1 invariants), so this does not refuse one — the Timeline can
    /// set a matte and delete its target without the document becoming invalid.
    #[frb(sync)]
    pub fn set_matte(&self, matte: Option<BridgeMatte>) -> Result<(), BridgeError> {
        use lumit_core::model::{LayerInputSource, MatteChannel, MatteRef};
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerMatte {
            comp,
            layer,
            matte: matte.map(|m| MatteRef {
                layer: m.layer,
                channel: if m.luma {
                    MatteChannel::Luma
                } else {
                    MatteChannel::Alpha
                },
                inverted: m.inverted,
                source: LayerInputSource::default(),
            }),
        })
    }

    /// This layer's transform parent, if any (K-103).
    #[frb(sync)]
    pub fn get_parent(&self) -> Result<Option<Uuid>, BridgeError> {
        Ok(self.item()?.parent)
    }

    /// Parent this layer to another, or clear it with `None`.
    ///
    /// A self-parent, an unknown layer, or one that would close a cycle is
    /// refused by the op — a parent loop has no defined transform, so unlike a
    /// dangling matte it cannot be allowed to exist and be ignored later.
    #[frb(sync)]
    pub fn set_parent(&self, parent: Option<Uuid>) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::SetLayerParent {
            comp,
            layer,
            parent,
        })
    }

    /// Move this layer to `new_index` in the stack (0 = top).
    #[frb(sync)]
    pub fn reorder(&self, new_index: usize) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.commit(lumit_core::Op::ReorderLayer {
            comp,
            layer,
            new_index,
        })
    }

    /// Remove this layer from its composition.
    #[frb(sync)]
    pub fn delete(&self) -> Result<(), BridgeError> {
        let (comp, layer) = (self.comp_id, self.layer_id);
        self.item()?;
        self.commit(lumit_core::Op::RemoveLayer { comp, layer })
    }

    /// Copy this layer, inserting the copy directly above the original.
    ///
    /// The copy is a fresh layer with fresh effect ids, not a second reference
    /// to the same one: two layers sharing an id would make every op that names
    /// a layer ambiguous.
    #[frb(sync)]
    pub fn duplicate(&self) -> Result<LayerReference, BridgeError> {
        let mut copy = self.item()?;
        let comp = self.composition()?;
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;

        copy.id = Uuid::now_v7();
        copy.name = format!("{} copy", copy.name);
        for effect in &mut copy.effects {
            effect.id = Uuid::now_v7();
        }
        // A duplicate of a layer that was somebody's matte or parent must not
        // inherit being pointed *at* — but it keeps what it points at itself.
        let new_id = copy.id;

        self.commit(lumit_core::Op::AddLayer {
            comp: self.comp_id,
            index,
            layer: Box::new(copy),
        })?;
        Ok(LayerReference::new(self.project_id, self.comp_id, new_id))
    }

    /// Commit `op` against this layer's project.
    #[frb(ignore)]
    pub(crate) fn commit(&self, op: lumit_core::Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// This layer's whole transform.
    #[frb(sync)]
    pub fn get_transform(&self) -> Result<BridgeTransform, BridgeError> {
        let layer = self.item()?;
        Ok(BridgeTransform::read_at(
            &layer.transform,
            layer.start_offset.0,
        ))
    }

    /// The layer's source audio summarised across `[start_seconds,
    /// end_seconds)` of the **layer's own clock**, in `buckets` buckets
    /// (K-280, superseding the fixed 2 048 of K-172).
    ///
    /// Layer time, not comp time, is what makes a trim or a drag free: the
    /// window is fixed to where the layer starts its source, so the Timeline's
    /// lane maps it through the live in/out/offset each paint and the
    /// transients travel with the bar. The *window* is what makes the
    /// resolution follow the zoom — a lane showing two seconds asks for two
    /// seconds, and gets a bucket per pixel column of them, however far in the
    /// Timeline is zoomed.
    ///
    /// **A retimed layer's wave stretches with its map** (K-436). Layer time
    /// and source time are the same line only while the layer plays at speed
    /// 1; once it has a Retime ([`lumit_core::model::Layer::source_time_at`])
    /// they are not, and buckets taken evenly in source time would put the
    /// transients in the wrong columns — a half-speed layer's wave would fill
    /// half its bar and stop. So each bucket's edges are mapped through that
    /// map here, exactly as a Sequence clip's are through its own
    /// ([`Self::clip_audio_peaks`]): the lane still draws bucket `i` at column
    /// `i`, and a slow passage is drawn wide because it *is* wide. An
    /// un-retimed layer maps through the identity and takes the straight,
    /// one-pass path it always did.
    ///
    /// `multiwave` asks for the three-band stack (bass, middle, treble) instead
    /// of the single full-range wave.
    ///
    /// Deliberately not `#[frb(sync)]`: the first ask for a file decodes it.
    /// Every later ask, at every zoom, is served from the session's peak cache
    /// ([`crate::peaks`]) and costs a walk over a few thousand summaries.
    /// Empty when the layer has no decodable audio.
    pub fn audio_peaks(
        &self,
        start_seconds: f64,
        end_seconds: f64,
        buckets: u32,
        multiwave: bool,
    ) -> Result<BridgeAudioPeaks, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Footage { item, .. } = layer.kind else {
            return Ok(BridgeAudioPeaks::empty());
        };

        #[cfg(feature = "media")]
        {
            // The read lock goes no further than resolving the path: building a
            // summary means decoding the file, and holding the project across
            // that stalls every other reader (docs/14 §3).
            let path = {
                let proj = self.project()?;
                let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
                let snapshot = proj.store.snapshot();
                let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item)
                else {
                    return Ok(BridgeAudioPeaks::empty());
                };
                crate::api::footage::FootageReference::resolve_path(&proj, footage)
            };
            let Some(path) = path else {
                return Ok(BridgeAudioPeaks::empty());
            };
            let Some(pyramid) = crate::peaks::pyramid_for(&path) else {
                return Ok(BridgeAudioPeaks::empty());
            };

            let buckets = buckets.min(MAX_PEAK_BUCKETS) as usize;
            // Where each bucket's edge lands in the source, through the
            // layer's Retime. One more edge than buckets, so neighbouring
            // buckets share theirs and no sliver of source falls between two
            // columns. `None` for an un-retimed layer: its map is the identity
            // line, so the pyramid can bucket the window itself in one pass.
            let edges: Option<Vec<f64>> = layer.retime.as_ref().map(|_| {
                let step = (end_seconds - start_seconds) / buckets.max(1) as f64;
                (0..=buckets)
                    .map(|i| layer.source_time_at(start_seconds + step * i as f64))
                    .collect()
            });
            let bands = BridgeAudioPeaks::bands_of(multiwave);
            let mut values = Vec::with_capacity(bands.len() * buckets * 3);
            for band in &bands {
                match &edges {
                    None => {
                        for block in pyramid.range(*band, start_seconds, end_seconds, buckets) {
                            values.extend_from_slice(&[block.min, block.max, block.rms]);
                        }
                    }
                    Some(edges) => {
                        for i in 0..buckets {
                            let (Some(&a), Some(&b)) = (edges.get(i), edges.get(i + 1)) else {
                                values.extend_from_slice(&[0.0, 0.0, 0.0]);
                                continue;
                            };
                            let block = pyramid.window(*band, a, b);
                            values.extend_from_slice(&[block.min, block.max, block.rms]);
                        }
                    }
                }
            }
            Ok(BridgeAudioPeaks {
                duration_seconds: pyramid.duration_seconds(),
                start_seconds,
                end_seconds,
                bands: bands.len() as u32,
                buckets: buckets as u32,
                values,
            })
        }

        #[cfg(not(feature = "media"))]
        {
            // Nothing decodes without FFmpeg, so the peaks are empty and the
            // lane draws nothing — the documented shape of a media-less build,
            // not a failure (docs/17 §Feature gates).
            let _ = (item, start_seconds, end_seconds, buckets, multiwave);
            Ok(BridgeAudioPeaks::empty())
        }
    }

    /// One Sequence clip's audio, summarised in `buckets` across the clip's own
    /// placed span — the waveform a clip draws inside itself (K-280).
    ///
    /// Bucketed in **clip-local placed time**, not source time, because a clip
    /// is the one thing on the timeline whose source clock is not a straight
    /// line: a ramp plays its middle slowly and its end fast, and buckets taken
    /// evenly in source time would put the transients in the wrong columns. So
    /// each bucket is mapped through the clip's own map here, where that map
    /// lives, and the lane draws bucket `i` at column `i` of the clip's box.
    /// Sliding the clip along the row moves the picture with it for free; a
    /// trim changes the mapping, so the lane asks again when the trim commits.
    ///
    /// `[start_seconds, end_seconds)` is the stretch of the clip's own placed
    /// clock to summarise, clamped to the clip; pass the clip's whole span for
    /// the whole clip, or the visible part of it to keep the detail level with
    /// the zoom. An empty or backwards range is read as the whole clip.
    ///
    /// `multiwave` asks for the three-band stack, exactly as for a layer. Empty
    /// for a clip cut from a comp or from media with no sound.
    pub fn clip_audio_peaks(
        &self,
        clip: Uuid,
        start_seconds: f64,
        end_seconds: f64,
        buckets: u32,
        multiwave: bool,
    ) -> Result<BridgeAudioPeaks, BridgeError> {
        let (clips, index) = self.clips_and_index(clip)?;
        let Some(clip) = clips.get(index) else {
            return Ok(BridgeAudioPeaks::empty());
        };
        let lumit_core::sequence::ClipSource::Footage(item) = clip.source else {
            return Ok(BridgeAudioPeaks::empty());
        };

        #[cfg(feature = "media")]
        {
            let path = {
                let proj = self.project()?;
                let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
                let snapshot = proj.store.snapshot();
                let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item)
                else {
                    return Ok(BridgeAudioPeaks::empty());
                };
                crate::api::footage::FootageReference::resolve_path(&proj, footage)
            };
            let Some(path) = path else {
                return Ok(BridgeAudioPeaks::empty());
            };
            let Some(pyramid) = crate::peaks::pyramid_for(&path) else {
                return Ok(BridgeAudioPeaks::empty());
            };

            let buckets = buckets.clamp(1, MAX_PEAK_BUCKETS) as usize;
            let clip_start = clip.place_start.to_f64();
            let clip_end = clip_start + clip.place_duration.to_f64();
            let (start, end) = if end_seconds > start_seconds {
                (
                    start_seconds.max(clip_start).min(clip_end),
                    end_seconds.max(clip_start).min(clip_end),
                )
            } else {
                (clip_start, clip_end)
            };
            if end <= start {
                return Ok(BridgeAudioPeaks::empty());
            }
            let step = (end - start) / buckets as f64;
            // Where each bucket's edge lands in the source, through the clip's
            // map. One more edge than buckets, so neighbouring buckets share
            // theirs and no sliver of source falls between two columns.
            let edges: Vec<f64> = (0..=buckets)
                .map(|i| clip.source_time(start + step * i as f64))
                .collect();
            let bands = BridgeAudioPeaks::bands_of(multiwave);
            let mut values = Vec::with_capacity(bands.len() * buckets * 3);
            for band in &bands {
                for i in 0..buckets {
                    let (Some(&a), Some(&b)) = (edges.get(i), edges.get(i + 1)) else {
                        values.extend_from_slice(&[0.0, 0.0, 0.0]);
                        continue;
                    };
                    let block = pyramid.window(*band, a, b);
                    values.extend_from_slice(&[block.min, block.max, block.rms]);
                }
            }
            Ok(BridgeAudioPeaks {
                duration_seconds: pyramid.duration_seconds(),
                start_seconds: start,
                end_seconds: end,
                bands: bands.len() as u32,
                buckets: buckets as u32,
                values,
            })
        }

        #[cfg(not(feature = "media"))]
        {
            let _ = (item, start_seconds, end_seconds, buckets, multiwave);
            Ok(BridgeAudioPeaks::empty())
        }
    }

    /// Whether this layer has a picture to sample — the mirror of
    /// [`Self::has_audio`], and what tells a matte or a layer-valued effect
    /// parameter which layers are worth offering (K-194).
    ///
    /// Every synthetic kind draws except the two that carry no pixels at all: a
    /// Camera (it *is* a viewpoint) and a Null (a transform and nothing else).
    /// Footage draws only when its container carries a video stream, so an
    /// audio-only clip answers false. An Audio layer (K-435) answers false
    /// whatever its file holds — that is what the flag means. Probing costs an
    /// FFmpeg open, so callers ask when a menu opens, never while drawing a row.
    #[frb(sync)]
    pub fn has_picture(&self) -> Result<bool, BridgeError> {
        use lumit_core::model::LayerKind as K;
        let layer = self.item()?;
        if layer.audio_only {
            return Ok(false);
        }
        let item = match layer.kind {
            K::Camera { .. } | K::Null => return Ok(false),
            K::Footage { item, .. } => item,
            // Solids, text, precomps, sequences and adjustments all draw.
            _ => return Ok(true),
        };

        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item) else {
            return Ok(false);
        };

        #[cfg(feature = "media")]
        {
            let Some(src) = crate::api::footage::FootageReference::resolve_source(&proj, footage)
            else {
                return Ok(false);
            };
            Ok(crate::probe::ensure_probed(&src)
                .map(|p| p.video.is_some())
                .unwrap_or(false))
        }

        // Without a decoder nothing can be probed. Footage is assumed to draw
        // rather than assumed not to: the opposite would empty every matte
        // menu on a build with no media feature.
        #[cfg(not(feature = "media"))]
        {
            let _ = footage;
            Ok(true)
        }
    }

    /// Whether this layer's source actually carries sound.
    ///
    /// What decides whether the Audio group and the mute switch appear under a
    /// layer at all (docs/07 §4.3): every layer *has* a Volume property in the
    /// model, but on a solid or a title it can never be heard, and a control
    /// that cannot do anything is worse than no control.
    ///
    /// **The mixer's own answer, at any depth.** A Precomp layer over footage
    /// that sings is audible — `walk` mixes it — and asking only whether *this*
    /// layer is Footage said no, so a converted precomp came up with no mute
    /// switch and no volume. [`AudioJobsBuilder::layer_has_audio`] is the one
    /// that decides what gets mixed, so it is the one asked here: the panel and
    /// the mixer cannot disagree about what makes a sound.
    ///
    /// Probing opens the file with FFmpeg, so this is deliberately **not**
    /// `#[frb(sync)]`, and the document lock is let go of before any probing.
    /// A layer whose media cannot be resolved answers false — a missing file is
    /// not a reason to offer a volume control.
    pub fn has_audio(&self) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let snapshot = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        // ponytail: a fresh probe cache per call, so a precomp of many footage
        // items is probed once per *ask* rather than once per session. The
        // frontend asks once per layer and holds the answer, so the ceiling is
        // one walk per layer; share the audio engine's builder if that ever
        // stops being true — it cannot be borrowed here without holding the
        // audio lock across a file open.
        Ok(lumit_render::headless::AudioJobsBuilder::new().layer_has_audio(&snapshot, &layer))
    }

    /// This layer's Retime property — layer-local time → source time, in
    /// seconds (K-197) — or `None` when the layer is not retimed, which is what
    /// hides the row.
    #[frb(sync)]
    pub fn get_retime_property(&self) -> Result<Option<BridgeScalar>, BridgeError> {
        let layer = self.item()?;
        Ok(layer
            .retime
            .as_ref()
            .map(|r| BridgeScalar::read_at(r, layer.start_offset.0)))
    }

    /// Turn Retime on or off (Ctrl+Alt+T), returning whether it is now on.
    ///
    /// On installs the identity map — source time running alongside local time
    /// — so switching it on changes nothing visible and gives the row something
    /// to key, exactly as AE's Time Remap does. Off removes the property
    /// rather than flattening it: "not retimed" and "retimed to exactly 1×" are
    /// different states in the file, and only the first skips the map.
    ///
    /// Off also re-hangs the layer on its source (K-212). A retimed layer can be
    /// any length, so when the map goes away the layer has to be given one
    /// again: it keeps its in point and the frame showing there, then plays at
    /// source rate until the source runs out or its own out point arrives,
    /// whichever comes first. It never grows. One undo step covers both.
    #[frb(sync)]
    pub fn toggle_retime_property(&self) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        // **A Sequence layer has no Retime of its own** (K-075): its clips
        // each carry one, edited in the sequence view, and a second map over
        // the whole row would be a rival to those — the very thing K-249
        // spent itself ending. Refused rather than quietly ignored, so the
        // menu and the chord can say why.
        if matches!(layer.kind, lumit_core::model::LayerKind::Sequence { .. }) {
            return Err(BridgeError::NotRetimeable);
        }
        let on = layer.retime.is_none();
        // The layer's own span in ITS time, which is where the two keys belong
        // (K-213): its comp in and out less where its zero sits. A layer that
        // has been moved or trimmed does not start at its own zero, and keys at
        // zero would sit at the start of the composition on screen and leave
        // the tail past `duration` frozen on one frame.
        let retime = on.then(|| {
            let local = |t: lumit_core::time::CompTime| {
                t.0.checked_sub(layer.start_offset.0).unwrap_or(t.0)
            };
            Layer::identity_retime(local(layer.in_point), local(layer.out_point))
        });
        let removal = lumit_core::Op::SetRetimeProperty {
            comp: self.comp_id,
            layer: self.layer_id,
            retime,
        };
        // Switching it off re-hangs the layer on its source (K-212); switching
        // it on changes nothing but the map.
        self.commit(if on {
            removal
        } else {
            self.unretime_op(&layer, removal)
        })?;
        Ok(on)
    }

    /// One op that switches a Retime off: `removal` — whichever of the two
    /// retime routes is being cleared — with the layer's span re-anchored on
    /// the frame that was showing, as a single undo step (K-212).
    ///
    /// Plain `removal` when the new span cannot be worked out (unreadable
    /// source time, or arithmetic that would overflow): switching Retime off
    /// must always work, even when nothing can be said about the source.
    #[frb(ignore)]
    pub(crate) fn unretime_op(&self, layer: &Layer, removal: lumit_core::Op) -> lumit_core::Op {
        let Some((in_point, out_point, start_offset)) = self.reanchored_span(layer) else {
            return removal;
        };
        lumit_core::Op::Batch {
            ops: vec![
                removal,
                lumit_core::Op::SetLayerSpan {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    in_point,
                    out_point,
                    start_offset,
                },
            ],
        }
    }

    /// Where this layer's span lands once its Retime goes away: anchored on the
    /// source moment showing at its in point, running at source rate until the
    /// source or its own out point ends it (`lumit_core::ops::unretimed_span`).
    ///
    /// The anchor is snapped to the **comp's** frame grid rather than kept at
    /// full precision, because the start offset it produces is what every later
    /// trim measures from: an offset sitting between two frames puts the
    /// layer's own zero between two frames for good, and the timeline edits in
    /// whole frames.
    #[frb(ignore)]
    fn reanchored_span(&self, layer: &Layer) -> Option<(CompTime, CompTime, CompTime)> {
        let rate = self.composition().ok()?.frame_rate;
        // The source moment showing at the in point, read through the map that
        // is about to be removed.
        let local = layer.in_point.0.checked_sub(layer.start_offset.0).ok()?;
        let seconds = layer.source_time_at(local.to_f64());
        let approximate = CompTime(Rational::from_f64_on_grid(seconds, Rational::FLICK_DEN).ok()?);
        let anchor = SourceTime(rate.time_of_frame(rate.frame_at(approximate)).ok()?.0);
        lumit_core::ops::unretimed_span(
            layer.in_point,
            layer.out_point,
            anchor,
            self.source_length(layer),
        )
    }

    /// How long this layer's source runs, when it has one that can be measured:
    /// a nested comp's duration, or a footage file's probed length. `None` for
    /// every generated kind, for media that will not read, and in builds
    /// without the `media` feature — "no length" is never a guessed length.
    #[frb(ignore)]
    fn source_length(&self, layer: &Layer) -> Option<Duration> {
        let proj = self.project().ok()?;
        let proj = proj.read().ok()?;
        let doc = proj.store.snapshot();
        match layer.kind {
            lumit_core::model::LayerKind::Precomp { comp } => match doc.item(comp) {
                Some(lumit_core::model::ProjectItem::Composition(inner)) => Some(inner.duration),
                _ => None,
            },
            #[cfg(feature = "media")]
            lumit_core::model::LayerKind::Footage { item, .. } => {
                let Some(lumit_core::model::ProjectItem::Footage(footage)) = doc.item(item) else {
                    return None;
                };
                let src = crate::api::footage::FootageReference::resolve_source(&proj, footage)?;
                let info = crate::probe::ensure_probed(&src)?;
                // The one sanctioned route back from the container's floating
                // point duration is an explicit grid (docs/impl/rational-time.md
                // §4) — the same millisecond grid `media_info` reports on.
                Some(Duration(
                    Rational::from_f64_on_grid(info.duration_seconds, 1000).ok()?,
                ))
            }
            _ => None,
        }
    }

    /// Replace the Retime property's whole animation, as one undoable step —
    /// the same coarse-grained shape as a transform property, for the same
    /// invertibility reason. Refused on a layer that is not retimed: the row
    /// only exists once it is.
    ///
    /// **A map that has become one constant takes the Retime away** rather than
    /// being written. Every route that produces one is the user saying "no more
    /// retime": the row's stopwatch turned off, or the last key deleted. Written
    /// as it arrived, a constant map is a layer frozen on a single frame for its
    /// whole length, with the row gone quiet and nothing on screen to say why —
    /// which is not a state K-197 has ("no freeze") and not what either gesture
    /// means. So it takes the Ctrl+Alt+T-off route instead: the property goes,
    /// and the layer is re-hung on its source at source rate (K-212), in one
    /// undo step.
    #[frb(sync)]
    pub fn set_retime_property(&self, value: BridgeScalar) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let animation = value.animation_at(layer.start_offset.0)?;
        let mut retime = layer.retime.clone().ok_or(BridgeError::NotRetimed)?;
        let removal = lumit_core::Op::SetRetimeProperty {
            comp: self.comp_id,
            layer: self.layer_id,
            retime: None,
        };
        if matches!(animation, lumit_core::anim::Animation::Static(_)) {
            return self.commit(self.unretime_op(&layer, removal));
        }
        retime.animation = animation;
        self.commit(lumit_core::Op::SetRetimeProperty {
            comp: self.comp_id,
            layer: self.layer_id,
            retime: Some(retime),
        })
    }

    /// The map this layer plays by, or the identity one it would play by if it
    /// were retimed — so a command that writes a map need not care which of the
    /// two states it started in.
    ///
    /// The identity runs over the layer's **own** span (K-213), the same two
    /// keys Ctrl+Alt+T installs.
    #[frb(ignore)]
    fn retime_or_identity(&self, layer: &Layer, from: Rational, to: Rational) -> Property {
        match &layer.retime {
            // A map that is one constant is not a map (see `set_retime_property`):
            // there is nothing to scale or split, so the identity stands in.
            Some(p) if matches!(p.animation, Animation::Keyframed(ref k) if k.len() > 1) => {
                p.clone()
            }
            _ => Layer::identity_retime(from, to),
        }
    }

    /// **Stretch** the layer (docs/04 §11.2): play it at `speed_percent` of the
    /// rate it plays at now, and give it the length that implies — 50% is half
    /// speed and twice as long.
    ///
    /// **Stretch is sugar over Retime** (K-584). It is not a second rate
    /// multiplier hiding behind the map: the map itself is rescaled, so the
    /// graph editor goes on showing the true curve and nothing about playback
    /// consults a stretch factor. That is the same lowering the After Effects
    /// importer already does with an imported layer's stretch, from the other
    /// direction.
    ///
    /// **Anchored at the in point**, which is the simplest honest default:
    /// After Effects offers three anchors (in point, current frame, out point)
    /// and Lumit offers the one that needs no extra question — the layer starts
    /// where it started and its end moves. The start offset is untouched, so
    /// the frame showing at the in point is the frame that stays there.
    ///
    /// The existing curve is kept: every key's time is scaled about the in
    /// point and every stored side speed is divided by the same factor, which
    /// is the shape said over a longer or shorter stretch rather than a new
    /// shape. A layer with no map yet gets the identity one first, so stretching
    /// an ordinary layer means what it looks like it means.
    ///
    /// One undo step — the span and the map move together or not at all.
    /// Refused on a Sequence layer (K-075: its clips carry the maps, and
    /// `set_clip_speed` is their road) and on a speed that is not a positive,
    /// finite number.
    #[frb(sync)]
    pub fn stretch(&self, speed_percent: f64) -> Result<(), BridgeError> {
        let layer = self.item()?;
        if matches!(layer.kind, lumit_core::model::LayerKind::Sequence { .. }) {
            return Err(BridgeError::NotRetimeable);
        }
        if !speed_percent.is_finite() || speed_percent <= 0.0 {
            return Err(BridgeError::InvalidTime);
        }
        let bad = |_| BridgeError::InvalidTime;
        // The factor the layer's *length* is multiplied by, which is the
        // reciprocal of the speed: at half speed it takes twice as long.
        let k =
            Rational::from_f64_on_grid(100.0 / speed_percent, Rational::FLICK_DEN).map_err(bad)?;
        let in_local = layer
            .in_point
            .0
            .checked_sub(layer.start_offset.0)
            .map_err(bad)?;
        let out_local = layer
            .out_point
            .0
            .checked_sub(layer.start_offset.0)
            .map_err(bad)?;
        let stretched = out_local
            .checked_sub(in_local)
            .and_then(|span| span.checked_mul(k))
            .and_then(|span| in_local.checked_add(span))
            .map_err(bad)?;
        let out_point = CompTime(layer.start_offset.0.checked_add(stretched).map_err(bad)?);

        let map = self.retime_or_identity(&layer, in_local, out_local);
        let Animation::Keyframed(keys) = &map.animation else {
            return Err(BridgeError::NotRetimed);
        };
        let kf = k.to_f64();
        // A stored side speed is value-units per **second** of local time, so
        // stretching the times divides it. Influence is a fraction of the span
        // and so survives untouched.
        let slower = |side: lumit_core::anim::SideInterp| match side {
            lumit_core::anim::SideInterp::Bezier { speed, influence } => {
                lumit_core::anim::SideInterp::Bezier {
                    speed: speed / kf,
                    influence,
                }
            }
            lumit_core::anim::SideInterp::Auto {
                clamped,
                speed,
                influence,
            } => lumit_core::anim::SideInterp::Auto {
                clamped,
                speed: speed / kf,
                influence,
            },
            other => other,
        };
        let mut scaled = Vec::with_capacity(keys.len());
        for key in keys {
            scaled.push(Keyframe {
                time: key
                    .time
                    .checked_sub(in_local)
                    .and_then(|d| d.checked_mul(k))
                    .and_then(|d| in_local.checked_add(d))
                    .map_err(bad)?,
                value: key.value,
                interp_in: slower(key.interp_in),
                interp_out: slower(key.interp_out),
            });
        }

        self.commit(lumit_core::Op::Batch {
            ops: vec![
                lumit_core::Op::SetLayerSpan {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    in_point: layer.in_point,
                    out_point,
                    start_offset: layer.start_offset,
                },
                lumit_core::Op::SetRetimeProperty {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    retime: Some(Property {
                        animation: Animation::Keyframed(scaled),
                        extra: map.extra.clone(),
                    }),
                },
            ],
        })
    }

    /// **Insert a freeze at the playhead** (docs/04 §7.3,
    /// `retime.freeze_at_playhead`): the moment showing at `frame` is held for
    /// one second, everything after it is pushed that far later, and the map is
    /// cropped back to the layer's own out point — so the layer's length never
    /// changes and the beat-sync covenant holds (K-022). The tail may newly
    /// overrun, which is drawn rather than repaired.
    ///
    /// One second is the specified default; the hold is two ordinary keyframes
    /// afterwards, so it is dragged like anything else.
    ///
    /// A layer with no map yet gets the identity one first — freezing a frame
    /// is a reasonable first retime to ask for, and refusing it because the
    /// stopwatch had not been touched would be a rule with no purpose.
    ///
    /// Refused on a Sequence layer (its clips carry the maps) and at a moment
    /// on or outside the layer's own ends, where there is nothing to split.
    #[frb(sync)]
    pub fn freeze_at_playhead(&self, frame: i64) -> Result<(), BridgeError> {
        use lumit_core::anim::SideInterp;

        let layer = self.item()?;
        if matches!(layer.kind, lumit_core::model::LayerKind::Sequence { .. }) {
            return Err(BridgeError::NotRetimeable);
        }
        let bad = |_| BridgeError::InvalidTime;
        let rate = self.composition()?.frame_rate;
        let at = rate.time_of_frame(frame).map_err(bad)?;
        let local = |t: CompTime| t.0.checked_sub(layer.start_offset.0).map_err(bad);
        let (in_local, out_local, at_local) =
            (local(layer.in_point)?, local(layer.out_point)?, local(at)?);
        if at_local <= in_local || at_local >= out_local {
            return Err(BridgeError::InvalidTime);
        }
        // The hold's length: one second of local time, per §7.3.
        let hold = Rational::new(1, 1).map_err(bad)?;
        let resume = at_local.checked_add(hold).map_err(bad)?;

        let mut map = self.retime_or_identity(&layer, in_local, out_local);
        // Split the curve where the freeze begins without changing it — the
        // razor's own move (K-221) — so the moment held is exactly the moment
        // that was showing.
        map.insert_key_preserving_shape(at_local);
        let Animation::Keyframed(keys) = &mut map.animation else {
            return Err(BridgeError::NotRetimed);
        };
        let split = at_local.to_f64();
        let Some(i) = keys
            .iter()
            .position(|k| (k.time.to_f64() - split).abs() < 1e-12)
        else {
            return Err(BridgeError::InvalidTime);
        };
        // What the curve did after this moment, handed to the key the freeze
        // ends on: the movement resumes where it left off rather than restarting
        // as a straight line.
        let resumes = keys[i].interp_out;
        let held = keys[i].value;
        keys[i].interp_out = SideInterp::Hold;
        for key in keys.iter_mut().skip(i + 1) {
            key.time = key.time.checked_add(hold).map_err(bad)?;
        }
        keys.insert(
            i + 1,
            Keyframe {
                time: resume,
                value: held,
                interp_in: SideInterp::Hold,
                interp_out: resumes,
            },
        );
        // Crop back to the layer's own out point: what the freeze pushed past
        // the end is gone, and the span straddling the end is split there so
        // the part that survives is the part that was already drawn.
        if keys.last().is_some_and(|k| k.time > out_local) {
            map.insert_key_preserving_shape(out_local);
            if let Animation::Keyframed(keys) = &mut map.animation {
                keys.retain(|k| k.time <= out_local);
            }
        }

        self.commit(lumit_core::Op::SetRetimeProperty {
            comp: self.comp_id,
            layer: self.layer_id,
            retime: Some(map),
        })
    }

    /// This layer's Volume, in dB (docs/09 §6): 0 is unity.
    #[frb(sync)]
    pub fn get_volume_db(&self) -> Result<BridgeScalar, BridgeError> {
        let layer = self.item()?;
        Ok(BridgeScalar::read_at(
            &layer.volume_db,
            layer.start_offset.0,
        ))
    }

    /// Set the Volume, as one undoable step — the same coarse-grained shape as
    /// a transform property, and for the same invertibility reason.
    #[frb(sync)]
    pub fn set_volume_db(&self, value: BridgeScalar) -> Result<(), BridgeError> {
        let animation = value.animation_at(self.item()?.start_offset.0)?;

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetLayerVolume {
                comp: self.comp_id,
                layer: self.layer_id,
                animation,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Replace several transform properties at once, as one undoable step.
    ///
    /// For a control that acts on a whole row: Position's stopwatch has to key
    /// x and y together, and two ops would be two undo steps for one click.
    /// They are separate properties in the model — that is what makes a
    /// per-axis curve possible — so a batch is how one gesture stays one step.
    ///
    /// An empty list is a no-op rather than an empty commit, so a caller need
    /// not check before calling.
    #[frb(sync)]
    pub fn set_transforms(
        &self,
        props: Vec<BridgeTransformProp>,
        values: Vec<BridgeScalar>,
    ) -> Result<(), BridgeError> {
        // Two parallel lists rather than a list of pairs: frb has no tuple, and
        // a struct for two fields used in one place is more ceremony than the
        // length check it saves.
        if props.len() != values.len() {
            return Err(BridgeError::MismatchedTransforms);
        }
        if props.is_empty() {
            return Ok(());
        }
        let offset = self.item()?.start_offset.0;

        let mut ops = Vec::with_capacity(props.len());
        for (prop, value) in props.into_iter().zip(values) {
            ops.push(lumit_core::Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop: prop.core(),
                animation: value.animation_at(offset)?,
            });
        }
        // One op stays one op; a batch of one would undo the same but reads
        // worse in the journal.
        let op = if ops.len() == 1 {
            ops.into_iter().next().ok_or(BridgeError::InvalidLayer)?
        } else {
            lumit_core::Op::Batch { ops }
        };
        self.commit(op)
    }

    /// Replace one transform property's whole animation, as one
    /// [`lumit_core::Op::SetTransformProperty`].
    ///
    /// One property per op, not the whole group: the op is exactly invertible
    /// that way, so a nudged Position is one undo step that puts back precisely
    /// what was there — where committing all eleven would make undo restore ten
    /// properties nobody touched.
    #[frb(sync)]
    pub fn set_transform(
        &self,
        prop: BridgeTransformProp,
        value: BridgeScalar,
    ) -> Result<(), BridgeError> {
        // Confirm the layer is there before committing, so a stale reference is
        // a calm error rather than a failed op — and its offset is what carries
        // the keys back onto its own clock (K-213).
        let animation = value.animation_at(self.item()?.start_offset.0)?;

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop: prop.core(),
                animation,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Set how one two-axis property is shown and edited (K-571): combined on
    /// one row, linked (Scale), or separated onto a row per axis.
    ///
    /// **Coming back together merges the axes' keyframes.** A separated pair's
    /// axes each keep their own keys; put back on one row they share a
    /// stopwatch and a lane, so every animated axis gains a key wherever any
    /// other animated axis in the pair has one. The planted keys take the value
    /// the curve already had and the spans around them are re-described, so the
    /// picture does not move — and a static axis is left static.
    ///
    /// The whole change is one [`lumit_core::Op::Batch`], so it is one undo
    /// step whatever it had to merge. Separating merges nothing: the axes are
    /// already stored apart, which is what makes a per-axis curve possible at
    /// all.
    #[frb(sync)]
    pub fn set_axis_mode(
        &self,
        pair: BridgeTransformPair,
        mode: BridgeAxisMode,
    ) -> Result<(), BridgeError> {
        let core_pair = pair.core();
        let mut ops = vec![lumit_core::Op::SetTransformAxisMode {
            comp: self.comp_id,
            layer: self.layer_id,
            pair: core_pair,
            mode: mode.core(),
        }];
        if mode != BridgeAxisMode::Separated {
            let layer = self.item()?;
            if layer.transform.axis_modes.get(core_pair) == lumit_core::model::AxisMode::Separated {
                for (prop, animation) in layer.transform.unified_axes(core_pair) {
                    ops.push(lumit_core::Op::SetTransformProperty {
                        comp: self.comp_id,
                        layer: self.layer_id,
                        prop,
                        animation,
                    });
                }
            }
        }
        let op = if ops.len() == 1 {
            ops.into_iter().next().ok_or(BridgeError::InvalidLayer)?
        } else {
            lumit_core::Op::Batch { ops }
        };
        self.commit(op)
    }

    #[frb(sync)]
    pub fn get_effects(&self) -> Result<Vec<BridgeEffectInstance>, BridgeError> {
        let layer = self.item()?;

        Ok(layer
            .effects
            .iter()
            .map(|f| BridgeEffectInstance::new(f.clone(), layer.start_offset.0))
            .collect())
    }

    /// Read this layer's effect stack, let `edit` change a clone of it, and
    /// commit the result as a single [`lumit_core::Op::SetLayerEffects`].
    ///
    /// The shared tail of every stack op below, exactly as v0's
    /// `edits::with_effects` is: one user action becomes one op and therefore one
    /// undo step (docs/17 "commands down"), and the two frontends cannot drift
    /// apart on what an effect edit means.
    ///
    /// Unlike v0 there is no drag overlay to discard here. The frb preview lives
    /// in the render request (`CompositionReference::render_frame_with_preview`)
    /// rather than in a field beside the document, so a failed edit cannot leave
    /// a stale staged value laid over later frames — the bug v0 had to clear the
    /// overlay at the top of `with_effects` to avoid.
    #[frb(ignore)]
    fn with_effects(
        &self,
        edit: impl FnOnce(&mut Vec<EffectInstance>) -> Result<(), BridgeError>,
    ) -> Result<(), BridgeError> {
        let mut effects = self.item()?.effects;
        edit(&mut effects)?;

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetLayerEffects {
                comp: self.comp_id,
                layer: self.layer_id,
                effects,
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Append the built-in effect named `name` to this layer's stack — or, when
    /// `name` is a **driver**, to this layer's graph.
    ///
    /// Seeded at composition size, because a few effects' defaults are positions
    /// (a transform's anchor and position start at the centre of the frame), and
    /// a fresh effect should look like identity rather than dragging the picture
    /// to a corner. An unknown name is refused; nothing partial is committed.
    ///
    /// **The fork is here rather than in each caller.** A driver is browsed as
    /// one more Controls entry, so the console, the Effect menu, the palette and
    /// the browser all offer one — and a driver on an effect *stack* would be a
    /// node that changes no pixel. Every one of those routes goes through this
    /// method, so the one question "is this a driver" is asked once and the node
    /// lands on the graph, unwired and unplaced (the panel auto-places a node
    /// with no layout entry). One op either way, so one undo step either way.
    #[frb(sync)]
    pub fn add_effect(&self, name: String) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        let mut instance = lumit_core::fx::instantiate_for_raster(
            &name,
            f64::from(comp.width),
            f64::from(comp.height),
        )
        .ok_or(BridgeError::UnknownEffectName)?;
        // A `self_default` layer reference starts pointed at the layer the
        // effect is landing on (K-288, docs/impl/layer-input.md): the Lens
        // flare's Matte source, whose natural reading is "the lights in this
        // picture" — and on an adjustment layer, the composite below.
        lumit_core::fx::point_self_layer_params_at(&mut instance, self.layer_id);

        if lumit_core::fx::BUILTIN_DEFS
            .get(&name)
            .is_some_and(|def| def.schema().category == lumit_core::fx::FxCategory::Drivers)
        {
            let mut graph = self.item()?.graph;
            graph.nodes.push(instance);
            let proj = self.project()?;
            let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
            proj.store
                .commit(lumit_core::Op::SetLayerGraph {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    graph: Box::new(graph),
                })
                .map_err(BridgeError::OpError)?;
            return Ok(());
        }

        self.with_effects(move |effects| {
            effects.push(instance);
            Ok(())
        })
    }

    /// Remove `effect` from this layer's stack. An effect that is no longer there
    /// is an error rather than a silent success, so a double-click on Remove
    /// cannot look as though it deleted a second effect.
    #[frb(sync)]
    pub fn remove_effect(&self, effect: &BridgeEffectInstance) -> Result<(), BridgeError> {
        let id = effect.id();
        self.with_effects(move |effects| {
            let before = effects.len();
            effects.retain(|e| e.id != id);
            if effects.len() == before {
                return Err(BridgeError::InvalidEffect);
            }
            Ok(())
        })
    }

    /// Move `effect` to `new_index` in the stack — drag-to-reorder.
    ///
    /// The index clamps into range rather than failing: past the end lands the
    /// effect at the bottom, negative lands it at the top. A drag that overshoots
    /// the list is an ordinary thing for a pointer to do, and refusing it would
    /// leave the effect where it started with no explanation.
    #[frb(sync)]
    pub fn reorder_effect(
        &self,
        effect: &BridgeEffectInstance,
        new_index: i64,
    ) -> Result<(), BridgeError> {
        let id = effect.id();
        self.with_effects(move |effects| {
            let from = effects
                .iter()
                .position(|e| e.id == id)
                .ok_or(BridgeError::InvalidEffect)?;
            let instance = effects.remove(from);
            let to = usize::try_from(new_index).unwrap_or(0).min(effects.len());
            effects.insert(to, instance);
            Ok(())
        })
    }

    /// Enable or bypass `effect`. A bypassed effect renders as identity and is
    /// not animatable (docs/08 §1.5 — the effect's own Mix parameter is the
    /// animatable dial).
    #[frb(sync)]
    pub fn set_effect_enabled(
        &self,
        effect: &BridgeEffectInstance,
        enabled: bool,
    ) -> Result<(), BridgeError> {
        let id = effect.id();
        self.with_effects(move |effects| {
            let instance = effects
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(BridgeError::InvalidEffect)?;
            instance.enabled = enabled;
            Ok(())
        })
    }

    /// Commit a staged effect stack — the mouse-up for a gesture Dart has been
    /// editing through `BridgeEffectInstance::set_value` and previewing through
    /// `CompositionReference::render_frame_with_preview`. The whole drag becomes
    /// one undo step, which is the entire point of staging (docs/17 ABI v12).
    ///
    /// Only parameter *values* may cross this way: the staged stack must still
    /// name the same effects, in the same order, as the document does. Otherwise
    /// a stack read before some other action removed an effect from it would
    /// resurrect that effect on mouse-up, and reordering or deleting would have
    /// two paths — this one, which cannot say what it meant, and the dedicated
    /// ops above, which can.
    #[frb(sync)]
    pub fn set_effects(&self, effects: Vec<BridgeEffectInstance>) -> Result<(), BridgeError> {
        let staged: Vec<EffectInstance> = effects
            .iter()
            .map(BridgeEffectInstance::get_effects)
            .collect();

        self.with_effects(move |current| {
            let same_stack = current.len() == staged.len()
                && current.iter().zip(&staged).all(|(a, b)| a.id == b.id);
            if !same_stack {
                return Err(BridgeError::StaleEffectStack);
            }
            *current = staged;
            Ok(())
        })
    }

    /// This layer's whole driver graph in one crossing (K-471,
    /// docs/impl/node-graph.md §5) — every box the canvas draws with its
    /// sockets, plus the wiring the user edits.
    ///
    /// **One call, not one per node.** Fetched when the selection or the
    /// document changes and held in Dart; asking per node per rebuild is
    /// exactly the traffic the budget test forbids (K-183). The boxes are
    /// derived from the effect stack each time, so there is nothing stale to
    /// invalidate and nothing here to write back — see
    /// [`crate::api::graph::BridgeLayerGraph`].
    #[frb(sync)]
    pub fn get_graph(&self) -> Result<crate::api::graph::BridgeLayerGraph, BridgeError> {
        Ok(crate::api::graph::read_layer_graph(&self.item()?))
    }

    /// This layer's driver nodes as **staged copies**, exactly as
    /// [`Self::get_effects`] hands out the stack's.
    ///
    /// A driver's parameters ride the ordinary property path from here:
    /// `BridgeEffectInstance::get_value` / `set_value` stage a change and
    /// [`Self::set_graph`] is the commit, so keyframing, the stopwatch and
    /// every existing property control work on a driver row unchanged.
    #[frb(sync)]
    pub fn get_graph_drivers(&self) -> Result<Vec<BridgeEffectInstance>, BridgeError> {
        let layer = self.item()?;
        Ok(layer
            .graph
            .nodes
            .iter()
            .map(|d| BridgeEffectInstance::new(d.clone(), layer.start_offset.0))
            .collect())
    }

    /// A new driver of the built-in named `name`, **uncommitted**.
    ///
    /// The mirror of [`Self::add_effect`] for the other kind of box, split in
    /// two because adding a driver is rarely the whole gesture: the panel drops
    /// the node, auto-wires it, places it, and commits all of that as one
    /// [`Self::set_graph`] — one op, one undo step (docs/impl/node-graph.md §3).
    /// An unknown name is refused; nothing is written either way.
    #[frb(sync)]
    pub fn new_driver(&self, name: String) -> Result<BridgeEffectInstance, BridgeError> {
        let layer = self.item()?;
        let comp = self.composition()?;
        let mut instance = lumit_core::fx::instantiate_for_raster(
            &name,
            f64::from(comp.width),
            f64::from(comp.height),
        )
        .ok_or(BridgeError::UnknownEffectName)?;
        // A `self_default` layer reference starts pointed at the layer the node
        // is landing on (K-288) — Audio level's Audio, on a footage layer, is
        // the sound of the thing you are wiring.
        lumit_core::fx::point_self_layer_params_at(&mut instance, self.layer_id);
        Ok(BridgeEffectInstance::new(instance, layer.start_offset.0))
    }

    /// Commit a whole graph: the staged driver nodes and the edited wiring, as
    /// one [`lumit_core::Op::SetLayerGraph`].
    ///
    /// The whole-graph shape is deliberate and mirrors `SetLayerEffects`. Add a
    /// driver, remove one, connect, disconnect, drag a box, toggle exposure —
    /// each is one write and therefore one undo step, and a delete takes its
    /// wires with it inside the same commit rather than leaving a dangling one
    /// behind.
    ///
    /// Unlike [`Self::set_effects`] the node list may differ from the
    /// document's: this *is* the structural op for drivers, there being no
    /// per-node one to defer to.
    ///
    /// A graph that breaks one of the model's rules is **refused**, not
    /// degraded — a wire to a missing node or port, a type mismatch, a second
    /// wire on one socket, or a loop among the drivers. Each arrives as
    /// `OpError::InvalidGraph` carrying the engine's own calm sentence, and a
    /// refused write leaves the document exactly as it was.
    #[frb(sync)]
    pub fn set_graph(
        &self,
        drivers: Vec<BridgeEffectInstance>,
        wiring: crate::api::graph::BridgeGraphWiring,
    ) -> Result<(), BridgeError> {
        let nodes: Vec<EffectInstance> = drivers
            .iter()
            .map(BridgeEffectInstance::get_effects)
            .collect();
        let graph = crate::api::graph::wiring_into(wiring, nodes);

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetLayerGraph {
                comp: self.comp_id,
                layer: self.layer_id,
                graph: Box::new(graph),
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// The JSON text of a **node group** gathered from `nodes` (K-651) — the
    /// mirror of `save_preset` for the graph canvas.
    ///
    /// The engine hands back the text and Dart chooses where it goes, exactly
    /// as an effect preset does: the engine never opens a file dialogue. What
    /// is saved is the driver boxes, the wires with both ends inside the set,
    /// and where they sat relative to one another; a wire leaving the set is
    /// not saved, because it names something the group does not carry.
    #[frb(sync)]
    pub fn save_node_group(
        &self,
        name: String,
        colour: u32,
        nodes: Vec<crate::api::graph::BridgeNodeRef>,
    ) -> Result<String, BridgeError> {
        let layer = self.item()?;
        let members: Vec<lumit_core::graph::NodeRef> =
            nodes.into_iter().map(|n| n.core()).collect();
        let preset = lumit_core::preset::group_from_graph(&layer.graph, &name, colour, &members);
        lumit_core::preset::group_to_json(&preset).map_err(|_| BridgeError::InvalidEffect)
    }

    /// Insert a saved node group at canvas point `(x, y)` — **one commit**, so
    /// one undo step however many boxes and wires it carries.
    ///
    /// Every instance id is minted here, so dropping one group twice never
    /// makes two boxes share an instance. Unlike `new_driver` this is not
    /// staged: dropping a saved rig *is* the whole gesture, there being nothing
    /// left to decide once the spot is chosen.
    #[frb(sync)]
    pub fn insert_node_group(&self, text: String, x: f64, y: f64) -> Result<(), BridgeError> {
        let preset =
            lumit_core::preset::group_from_json(&text).map_err(|_| BridgeError::InvalidEffect)?;
        let added = lumit_core::preset::group_instantiated(&preset, [x, y]);

        let mut graph = self.item()?.graph;
        graph.nodes.extend(added.nodes);
        graph.edges.extend(added.edges);
        graph.layout.extend(added.layout);
        graph.groups.push(added.group);

        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetLayerGraph {
                comp: self.comp_id,
                layer: self.layer_id,
                graph: Box::new(graph),
            })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Which of this layer's property groups the reveal shortcuts should open
    /// (docs/07 §4.3's `U` / `UU`, K-199).
    ///
    /// The *question* is the engine's, which is why it is answered here rather
    /// than in the Timeline: "does this group hold anything animated" and "is
    /// any of it changed from what a fresh layer would have" are facts about
    /// the document, and the second needs the seeding rule
    /// ([`crate::edits::centred_transform`]) that decides what "unchanged"
    /// means for Position. The panel is told which groups to open; it decides
    /// nothing about why.
    #[frb(sync)]
    pub fn reveal_groups(&self, kind: BridgeRevealKind) -> Result<BridgeRevealGroups, BridgeError> {
        let layer = self.item()?;
        let comp = self.composition()?;
        let animated = |p: &lumit_core::anim::Property| {
            matches!(p.animation, lumit_core::anim::Animation::Keyframed(_))
        };

        // What this layer's transform would be had nobody touched it. Natural
        // size is not known here (it is the source's), and it only seeds the
        // anchor, so the anchor is compared loosely: an anchor that is not the
        // model default counts as modified only when it is also not a plain
        // half-size, which is the shape the seeding gives it.
        let fresh = crate::edits::centred_transform(0.0, 0.0, comp.width, comp.height);
        let t = &layer.transform;
        let default_t = lumit_core::model::TransformGroup::default();
        let transform_props: [(&lumit_core::anim::Property, &lumit_core::anim::Property); 11] = [
            (&t.anchor_x, &default_t.anchor_x),
            (&t.anchor_y, &default_t.anchor_y),
            (&t.position_x, &fresh.position_x),
            (&t.position_y, &fresh.position_y),
            (&t.scale_x, &default_t.scale_x),
            (&t.scale_y, &default_t.scale_y),
            (&t.rotation, &default_t.rotation),
            (&t.position_z, &default_t.position_z),
            (&t.rotation_x, &default_t.rotation_x),
            (&t.rotation_y, &default_t.rotation_y),
            (&t.opacity, &default_t.opacity),
        ];
        let transform = match kind {
            BridgeRevealKind::Animated => transform_props.iter().any(|(p, _)| animated(p)),
            BridgeRevealKind::Modified => transform_props
                .iter()
                // The anchor is exempt from the "differs from default" half:
                // its seeded value depends on the source's natural size, which
                // is not the document's to know here, so a footage layer would
                // otherwise always read as modified.
                .enumerate()
                .any(|(i, (p, d))| animated(p) || (i > 1 && p.animation != d.animation)),
        };

        // An effect qualifies when a parameter of it is keyframed; for
        // Modified, its mere presence qualifies it — a layer with an effect on
        // it has been modified, whatever the parameters say.
        let effects: Vec<String> = layer
            .effects
            .iter()
            .filter(|fx| match kind {
                BridgeRevealKind::Modified => true,
                BridgeRevealKind::Animated => fx.params.iter().any(|p| match &p.value {
                    lumit_core::model::EffectValue::Float(prop) => animated(prop),
                    lumit_core::model::EffectValue::Point(x, y) => animated(x) || animated(y),
                    lumit_core::model::EffectValue::Colour(ch) => ch.iter().any(animated),
                    _ => false,
                }),
            })
            .map(|fx| fx.id.to_string())
            .collect();

        let volume_default = lumit_core::anim::Property::fixed(0.0);
        let audio = match kind {
            BridgeRevealKind::Animated => animated(&layer.volume_db),
            BridgeRevealKind::Modified => {
                animated(&layer.volume_db) || layer.volume_db.animation != volume_default.animation
            }
        };

        // Retime is a row, not a group, and it is only ever there because
        // somebody switched it on — so its presence is a modification, and its
        // keys make it animated.
        let retime = match (&layer.retime, kind) {
            (None, _) => false,
            (Some(_), BridgeRevealKind::Modified) => true,
            (Some(p), BridgeRevealKind::Animated) => animated(p),
        };

        Ok(BridgeRevealGroups {
            transform,
            audio,
            retime,
            any: transform || audio || retime || !effects.is_empty(),
            effects,
        })
    }
}

/// Which reveal the Timeline is asking for (docs/07 §4.3).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRevealKind {
    /// `U`: groups holding a keyframed property.
    Animated,
    /// `UU`: groups holding anything changed from a fresh layer's state.
    Modified,
}

/// The groups a reveal should open on one layer. Effects are named
/// individually, because the Effects group opens onto one row per effect and
/// only the qualifying ones should unfold.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRevealGroups {
    pub transform: bool,
    pub audio: bool,
    pub retime: bool,
    /// The qualifying effects' ids, as text — the same form the fold paths use.
    pub effects: Vec<String>,
    /// Whether anything qualified at all. The panel leaves the layer's own
    /// twirl shut when nothing did, rather than opening onto an empty list.
    pub any: bool,
}

/// The last source position a map reaches — what the clip asks of its source.
#[frb(ignore)]
fn map_end_value(map: &lumit_core::anim::Property) -> Option<lumit_core::time::Rational> {
    let lumit_core::anim::Animation::Keyframed(keys) = &map.animation else {
        return None;
    };
    let last = keys.last()?;
    lumit_core::time::Rational::from_f64_on_grid(last.value, lumit_core::time::Rational::FLICK_DEN)
        .ok()
}
