//! Shape layers: vector art as a layer's own picture (docs/03-DATA-MODEL.md
//! §7.2, docs/impl/shape-layers.md), and the rasteriser that draws one.
//!
//! # In plain terms
//!
//! A **mask** is a path drawn *on* another layer, deciding which of its pixels
//! show. A **shape layer** is a path that **is** the picture: a rectangle, an
//! ellipse, a drawn path — filled, outlined, and made of numbers rather than
//! pixels, so it stays crisp at any size. After Effects makes one whenever you
//! drag a shape tool with nothing selected, and this is the layer kind that lets
//! Lumit do the same.
//!
//! **The path type is the mask's.** One `BezierPath` in the document, one set of
//! maths, one vertex type crossing the bridge. A shape's path and a mask's path
//! differ in what they *do*, not in what they are — which is why the shape tools
//! could draw both from the same geometry from the day they landed (K-222).
//!
//! **The layer's own size is the art's bounding box**, and it changes as the art
//! is edited. Every other layer kind has a size fixed by its source; this is the
//! first that does not, and anything caching "how big is this layer" has to
//! follow the document's revision rather than assume (docs/impl/shape-layers.md).
//!
//! **The modifiers are fields on the item, not a tree** (K-551). After Effects
//! carries Trim Paths, the Repeater and the rest as entries in a nested group,
//! where their position decides what they act on. Lumit's list is flat, so each
//! modifier is a property of the item it modifies and the order they apply in is
//! fixed and written down here rather than dragged: **trim, then repeat**. The
//! nested tree is still the long-term shape (docs/03 §9.2); nothing stored here
//! stands in its way, because every modifier is absent from the file until it is
//! used.
//!
//! **What is deliberately not here.** Nested groups, wiggle paths, line joins
//! other than round, and animated paths.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::anim::Property;
use crate::mask::{BezierPath, Vertex};
use crate::model::LinearColour;
use crate::pixels::over;

/// One piece of vector art: a path, and how it is painted.
///
/// A flat list of these per layer rather than After Effects' nested groups —
/// groups are a modifier feature and are later work (docs/impl/shape-layers.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeItem {
    pub id: Uuid,
    pub name: String,
    pub path: BezierPath,
    /// The colour inside the path. `None` draws no fill, which is how an
    /// outline-only shape is made.
    #[serde(default)]
    pub fill: Option<LinearColour>,
    /// The colour of the outline, and how wide it is in layer pixels. `None`
    /// draws no stroke.
    #[serde(default)]
    pub stroke: Option<LinearColour>,
    #[serde(default)]
    pub stroke_width: f64,
    /// 0..100, like every other opacity in the document.
    #[serde(default = "full_opacity")]
    pub opacity: f64,
    /// **Trim paths** (K-551): where along the path the art begins and ends, as
    /// a per cent of the path's own **arc length**, and how far the pair is slid
    /// along it in degrees (360 is once round).
    ///
    /// Per cent of length rather than of vertex count, for the reason a paint
    /// stroke's write-on gives (K-549): the eye watches length. The trim cuts
    /// the fill as well as the outline — a half-trimmed circle is a half circle,
    /// filled by closing the piece that is left.
    ///
    /// `end` at or below `start` draws nothing at all, which is what the first
    /// frame of a write-on looks like rather than an error.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub trim_start: Property,
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub trim_end: Property,
    /// Degrees; 360 slides the trimmed piece exactly once round a closed path.
    /// A closed path **wraps** — the piece runs through the seam and comes back
    /// — and an open one does not, because it has no seam to run through.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub trim_offset: Property,
    /// **Dashes** (K-552): the outline's dash and gap lengths in layer pixels,
    /// alternating — dash, gap, dash, gap — and `dash_offset` is how far along
    /// the path the pattern starts, in the same pixels.
    ///
    /// Empty is a **solid** outline, which is what every stroke was before there
    /// were dashes, and is absent from the file. An odd-length list repeats
    /// itself to make an even one, which is the SVG rule and the only reading
    /// that does not leave a dash with no gap after it.
    #[serde(
        default,
        with = "crate::mask::still_or_keyed_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub dashes: Vec<Property>,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub dash_offset: Property,
    /// **A gradient fill** (K-555): 0 draws the flat [`fill`](Self::fill), 1
    /// ramps from it **linearly** to [`gradient_colour`](Self::gradient_colour)
    /// and 2 ramps **radially**, `gradient_colour` sitting on the outer edge.
    ///
    /// A choice rather than a `Property`: what a ramp *is* does not tween, and
    /// a number between linear and radial would have to mean something. The
    /// two ends are colours like the fill beside them, and the two points that
    /// aim the ramp are `Property` and animate.
    #[serde(default, skip_serializing_if = "is_flat")]
    pub gradient: u32,
    /// The colour at the far end of the ramp. `None` is black, which is where
    /// a gradient nobody has picked a second colour for starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_colour: Option<LinearColour>,
    /// Where the ramp starts and ends, in **layer pixels** — the art's own
    /// coordinates, the same ones its vertices are in. Linear projects onto
    /// the line between them; radial measures out from the start, the end
    /// sitting on the outer edge.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub gradient_start_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub gradient_start_y: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub gradient_end_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub gradient_end_y: Property,
    /// **Offset paths** (K-554): how far the outline is pushed **out** of the
    /// path, in layer pixels — negative pulls it in. Zero is the path itself
    /// and is absent from the file.
    ///
    /// The corners are **round**, which is the one join this crate draws
    /// (K-237); the offset does not undo its own self-intersections, so an
    /// inward offset past a curve's own radius leaves a small loop that the
    /// non-zero winding fill mostly swallows.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub offset_amount: Property,
    /// **The repeater** (K-553): how many copies of the item are drawn, and the
    /// transform each copy is one more step of.
    ///
    /// `repeat_copies` is a count, rounded and held to 1..[`MAX_COPIES`]; a
    /// still 1 is "no repeater" and is what every shape is until somebody asks
    /// for more. `repeat_offset` says which copy the original is: copy *k* runs
    /// from the offset, so a negative offset puts copies *behind* the original.
    #[serde(
        default = "one",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_one"
    )]
    pub repeat_copies: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub repeat_offset: Property,
    /// The point the copy transform turns and scales about, in layer pixels.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub repeat_anchor_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub repeat_anchor_y: Property,
    /// How far one copy is moved from the one before it, in layer pixels.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub repeat_position_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub repeat_position_y: Property,
    /// How far one copy is turned from the one before it, in degrees.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub repeat_rotation: Property,
    /// How much one copy is scaled from the one before it, per cent — 100 being
    /// the same size.
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub repeat_scale: Property,
    /// The opacity of the first and last copy, per cent; the ones between ramp
    /// evenly from one to the other.
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub repeat_start_opacity: Property,
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub repeat_end_opacity: Property,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn full_opacity() -> f64 {
    100.0
}

/// serde default for [`ShapeItem::repeat_copies`]: one copy, which is no
/// repeater at all.
fn one() -> Property {
    Property::fixed(1.0)
}

/// True for a flat fill — no gradient, and so nothing written to the file.
fn is_flat(kind: &u32) -> bool {
    *kind == 0
}

/// True for a still 1 — the repeater switched off, and so the thing left out of
/// the file entirely.
fn is_static_one(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 1.0) && p.extra.is_empty()
}

/// The most copies one repeated item is drawn as.
///
/// Every copy is a rasteriser pass of its own over the whole layer, so the
/// count is the frame's cost written as a number. A hundred is past what a row
/// of things or a ring of ticks asks for and still inside a frame; a count past
/// it is **held** here rather than refused, because the number is a slider and a
/// slider that stops is kinder than a frame that does not arrive. Lifting the
/// ceiling means a rasteriser that can be pointed at one copy's own box rather
/// than the whole layer, which is a change to the mask rasteriser, not here.
pub const MAX_COPIES: i64 = 100;

impl ShapeItem {
    /// A filled item with no outline — what a shape tool makes.
    pub fn filled(name: impl Into<String>, path: BezierPath, fill: LinearColour) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            path,
            fill: Some(fill),
            stroke: None,
            stroke_width: 0.0,
            opacity: 100.0,
            trim_start: Property::zero(),
            trim_end: crate::paint::full(),
            trim_offset: Property::zero(),
            dashes: Vec::new(),
            dash_offset: Property::zero(),
            gradient: 0,
            gradient_colour: None,
            gradient_start_x: Property::zero(),
            gradient_start_y: Property::zero(),
            gradient_end_x: Property::zero(),
            gradient_end_y: Property::zero(),
            offset_amount: Property::zero(),
            repeat_copies: one(),
            repeat_offset: Property::zero(),
            repeat_anchor_x: Property::zero(),
            repeat_anchor_y: Property::zero(),
            repeat_position_x: Property::zero(),
            repeat_position_y: Property::zero(),
            repeat_rotation: Property::zero(),
            repeat_scale: crate::paint::full(),
            repeat_start_opacity: crate::paint::full(),
            repeat_end_opacity: crate::paint::full(),
            extra: serde_json::Map::new(),
        }
    }

    /// True when the trim at `t` would change the art — false for the whole
    /// path, which is the state every shape is in until somebody trims it.
    ///
    /// The check exists so an untrimmed shape is rasterised from its **bezier**
    /// rather than from a polyline of it: flattening is exact enough to stroke
    /// with and not exact enough to make the identity case draw different
    /// pixels than it did before there were modifiers.
    fn trims_at(&self, t: f64) -> bool {
        self.trim_start.value_at(t) > 0.0
            || self.trim_end.value_at(t) < 100.0
            || self.trim_offset.value_at(t) != 0.0
    }

    /// The item's art at `t` as a polyline, once the offset and the trim have
    /// had it — `None` when neither of them changes the path (draw the bezier
    /// instead), and `Some(empty)` when the trim leaves nothing at all.
    ///
    /// The order is **offset, then trim** (docs/03 §7.2.1): the offset makes
    /// the outline, and the trim cuts whatever outline there is by its length.
    fn trimmed_at(&self, t: f64) -> Option<Vec<(f64, f64)>> {
        let amount = self.offset_amount.value_at(t);
        if !self.trims_at(t) && amount == 0.0 {
            return None;
        }
        let mut points = flatten_path(&self.path);
        if points.len() < 2 {
            return None;
        }
        if amount != 0.0 {
            points = offset_polyline(&points, amount, self.path.closed);
            if points.len() < 2 {
                return Some(points);
            }
        }
        if !self.trims_at(t) {
            return Some(points);
        }
        let (start, end) = (
            self.trim_start.value_at(t).clamp(0.0, 100.0),
            self.trim_end.value_at(t).clamp(0.0, 100.0),
        );
        // Degrees to per cent of the path: 360 is once round.
        let shift = self.trim_offset.value_at(t) / 3.6;
        Some(if self.path.closed {
            // A closed path has a seam to run through, so the offset moves the
            // seam rather than the window: re-start the polyline `shift` per
            // cent along, and the ordinary trim then cuts one contiguous piece
            // even when it straddles where the first vertex used to be.
            crate::paint::trimmed(&rotated(&points, shift), start, end)
        } else {
            // An open path has two ends. Sliding the window off either of them
            // is the window running out of path, which is what clamping says.
            crate::paint::trimmed(&points, start + shift, end + shift)
        })
    }

    /// The gradient this item's fill is drawn with at `t`, placed by `to_box`
    /// (a repeated copy carries its gradient with it) and scaled into the
    /// buffer's pixels — `None` for the flat fill every shape has until
    /// somebody ramps it.
    fn ramp_at(&self, t: f64, to_box: &Affine, sx: f64, sy: f64) -> Option<Ramp> {
        if self.gradient == 0 {
            return None;
        }
        let from = self.fill?;
        let to = self
            .gradient_colour
            .unwrap_or(crate::model::LinearColour::BLACK);
        let place = |x: f64, y: f64| {
            let (x, y) = to_box.apply((x, y));
            (x * sx, y * sy)
        };
        let start = place(
            self.gradient_start_x.value_at(t),
            self.gradient_start_y.value_at(t),
        );
        let end = place(
            self.gradient_end_x.value_at(t),
            self.gradient_end_y.value_at(t),
        );
        let axis = (end.0 - start.0, end.1 - start.1);
        // One epsilon on the squared length, so the linear and the radial
        // reading degenerate at exactly the same point — the Gradient effect's
        // rule, and for the same reason.
        let len2 = (axis.0 * axis.0 + axis.1 * axis.1).max(1e-6);
        let mut lut = [[0u8; 4]; RAMP_STEPS];
        for (i, cell) in lut.iter_mut().enumerate() {
            let u = i as f32 / (RAMP_STEPS - 1) as f32;
            let mix = crate::model::LinearColour([
                from.0[0] + (to.0[0] - from.0[0]) * u,
                from.0[1] + (to.0[1] - from.0[1]) * u,
                from.0[2] + (to.0[2] - from.0[2]) * u,
                from.0[3] + (to.0[3] - from.0[3]) * u,
            ]);
            *cell = crate::pixels::solid_rgba(mix);
        }
        Some(Ramp {
            radial: self.gradient == 2,
            start,
            axis,
            inv_len2: 1.0 / len2,
            inv_len: 1.0 / len2.sqrt(),
            lut,
        })
    }

    /// The dash pattern at `t` in layer pixels, evened out — empty when the
    /// outline is solid, which is the ordinary case.
    ///
    /// An odd-length list is repeated (the SVG rule): `[10]` is ten on, ten off,
    /// not ten on and nothing said about the rest. A pattern whose lengths are
    /// all zero is no pattern at all.
    fn dash_pattern_at(&self, t: f64) -> Vec<f64> {
        let mut pattern: Vec<f64> = self.dashes.iter().map(|p| p.value_at(t).max(0.0)).collect();
        if pattern.iter().all(|&d| d <= 0.0) {
            return Vec::new();
        }
        if !pattern.len().is_multiple_of(2) {
            pattern.extend_from_within(..);
        }
        pattern
    }

    /// The copies this item is drawn as at `t`, each with the transform to
    /// apply to its geometry and the share of its opacity to draw it at (0..1,
    /// the item's *own* opacity not in it — the fill and the outline apply that
    /// where they always did).
    ///
    /// One copy, identity, full opacity, for an item nobody has repeated —
    /// which is the same single draw a shape has always been.
    fn copies_at(&self, t: f64) -> Vec<(Affine, f64)> {
        let copies = self
            .repeat_copies
            .value_at(t)
            .round()
            .clamp(1.0, MAX_COPIES as f64) as i64;
        if copies <= 1 {
            return vec![(Affine::IDENTITY, 1.0)];
        }
        let offset = self
            .repeat_offset
            .value_at(t)
            .round()
            .clamp(-(MAX_COPIES as f64), MAX_COPIES as f64) as i64;
        let step = repeat_step(
            (
                self.repeat_position_x.value_at(t),
                self.repeat_position_y.value_at(t),
            ),
            self.repeat_rotation.value_at(t),
            self.repeat_scale.value_at(t) / 100.0,
            (
                self.repeat_anchor_x.value_at(t),
                self.repeat_anchor_y.value_at(t),
            ),
        );
        let (from, to) = (
            self.repeat_start_opacity.value_at(t).clamp(0.0, 100.0) / 100.0,
            self.repeat_end_opacity.value_at(t).clamp(0.0, 100.0) / 100.0,
        );

        // The first copy is step^offset, and every one after it is one more
        // step — so the offset decides which copy the original geometry is, and
        // a negative one puts copies *behind* it. Stepping the accumulator
        // rather than raising the transform to a power each time keeps the work
        // one multiplication per copy.
        let (walk, back) = (step, step.inverse());
        let mut m = Affine::IDENTITY;
        for _ in 0..offset.abs() {
            m = m.then(if offset >= 0 { &walk } else { &back });
        }

        (0..copies)
            .map(|j| {
                let here = m;
                m = m.then(&walk);
                // The ramp runs across the copies drawn, not across the offset:
                // it is "the first one is this bright, the last one is that",
                // which is what the two numbers say.
                let ramp = from + (to - from) * (j as f64 / (copies - 1) as f64);
                (here, ramp)
            })
            .collect()
    }

    /// The item's bounding box in layer coordinates at `t`, its outline and its
    /// repeated copies included, or `None` when its path has no vertices.
    ///
    /// The **control points** bound the curve rather than the curve itself: a
    /// cubic never leaves its own control hull, so this is correct and never
    /// too small. It can be a little generous on a strongly curved path, which
    /// costs a few transparent pixels and no correctness.
    ///
    /// `t` is here because the **repeater** puts art where the path is not
    /// (K-553), and where it puts it can be keyed. A trim needs no clock — it
    /// only ever takes art away, and the box stays the untrimmed one for the
    /// reason a paint stroke's bounds give (K-549).
    pub fn bounds(&self, t: f64) -> Option<(f64, f64, f64, f64)> {
        let mut out: Option<(f64, f64, f64, f64)> = None;
        for v in &self.path.vertices {
            for (x, y) in [
                v.pos,
                (v.pos.0 + v.tan_in.0, v.pos.1 + v.tan_in.1),
                (v.pos.0 + v.tan_out.0, v.pos.1 + v.tan_out.1),
            ] {
                out = Some(match out {
                    None => (x, y, x, y),
                    Some(b) => (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y)),
                });
            }
        }
        let (x0, y0, x1, y1) = out?;
        // Half the stroke sits outside the path, so the box has to hold it.
        // Inflating **before** the copies are placed is what makes a scaled
        // copy's outline fit too: the copy's transform grows the margin with
        // the art.
        let half = if self.stroke.is_some() {
            self.stroke_width.max(0.0) / 2.0
        } else {
            0.0
        };
        // An outline pushed out of the path is art outside the path, so the
        // box holds it too (K-554). Pulled *in* it never needs more room, so
        // only the outward half counts.
        let half = half + self.offset_amount.value_at(t).max(0.0);
        let (x0, y0, x1, y1) = (x0 - half, y0 - half, x1 + half, y1 + half);

        // Every copy's box, unioned. One copy and the identity — every shape
        // nobody has repeated — hands back exactly the box above.
        let mut whole: Option<(f64, f64, f64, f64)> = None;
        for (m, _) in self.copies_at(t) {
            for corner in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
                let (x, y) = m.apply(corner);
                whole = Some(match whole {
                    None => (x, y, x, y),
                    Some(b) => (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y)),
                });
            }
        }
        whole
    }
}

/// The bounding box of a whole layer's worth of art at `t`, or `None` when
/// there is no art in it — the layer's **natural size** (docs/06 §1.2 step 1).
///
/// This is the number the wireframe, hit-testing and the transform all read, and
/// it moves when the art is edited: a shape layer is the first kind whose size
/// is not fixed by its source. Since K-553 it can also move as the art is
/// *played*, because a keyed repeater puts copies somewhere new each frame.
pub fn contents_bounds(contents: &[ShapeItem], t: f64) -> Option<(f64, f64, f64, f64)> {
    let mut out: Option<(f64, f64, f64, f64)> = None;
    for item in contents {
        let Some(b) = item.bounds(t) else { continue };
        out = Some(match out {
            None => b,
            Some(a) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
        });
    }
    out
}

/// Draw `contents` into a fresh `w`×`h` RGBA buffer whose extent is the box
/// `(min_x, min_y)`–`(max_x, max_y)` in layer coordinates.
///
/// Items are drawn in order, each filled and then stroked, so an item's own
/// outline sits over its own fill. The buffer starts transparent: a shape layer
/// is its art and nothing else.
///
/// The fill's coverage comes from the **mask rasteriser** — the same scanline
/// walk, with the same two vertical subsamples, that decides which pixels a mask
/// gates. One path type, one rasteriser.
///
/// `t` is the **layer's** own clock (K-213), the one its masks and its paint are
/// read on: it is what a keyframed trim is sampled at.
// The box is four numbers and the raster is two; bundling them into a struct
// nobody else holds would be a name for an argument list, not a type.
#[allow(clippy::too_many_arguments)]
pub fn rasterise_contents(
    contents: &[ShapeItem],
    w: u32,
    h: u32,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    t: f64,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    if w == 0 || h == 0 {
        return rgba;
    }
    let sx = f64::from(w) / (max_x - min_x).max(1e-9);
    let sy = f64::from(h) / (max_y - min_y).max(1e-9);

    for item in contents {
        let opacity = (item.opacity.clamp(0.0, 100.0) / 100.0) as f32;
        if opacity <= 0.0 {
            continue;
        }
        // The art at this instant, once the modifiers have had it. `None` is
        // "nothing has changed it", which draws the bezier itself.
        let trimmed = item.trimmed_at(t);
        if trimmed.as_ref().is_some_and(|p| p.len() < 2) {
            continue; // trimmed away to nothing: the first frame of a write-on
        }

        // The copies the repeater asks for (K-553), drawn **last first** so the
        // original ends up on top of the copies made from it — which is what
        // After Effects draws, and the only order in which turning the count up
        // does not hide the shape you already had.
        for (copy, ramp) in item.copies_at(t).into_iter().rev() {
            let opacity = opacity * ramp as f32;
            if opacity <= 0.0 {
                continue;
            }
            // The copy's own transform and the shift into the buffer's
            // coordinates, composed into one: the rasteriser works from the
            // origin, so the art is moved to the box's top-left as it is
            // placed. An item nobody has repeated is placed by the identity, so
            // this is exactly the subtraction it has always been.
            let to_box = copy.then(&Affine::translation(-min_x, -min_y));
            let placed = match &trimmed {
                // A trimmed piece is a polyline, and a polyline is a bezier
                // whose handles are all zero — one path type, still (K-237).
                // Closed so the fill has something to fill: a half-trimmed
                // circle fills as a half circle, exactly as AE draws it.
                Some(points) => polyline_path(
                    &points.iter().map(|&p| to_box.apply(p)).collect::<Vec<_>>(),
                    item.path.closed,
                ),
                None => transform_path(&item.path, &to_box),
            };

            if let Some(fill) = item.fill {
                let coverage = crate::mask::rasterise(&placed, w, h, sx, sy);
                // A gradient fill (K-555) is the same coverage painted with a
                // colour that changes across it; the flat fill is the same
                // walk with the colour worked out once.
                match item.ramp_at(t, &to_box, sx, sy) {
                    None => {
                        let colour = crate::pixels::solid_rgba(fill);
                        let alpha = f32::from(colour[3]) / 255.0;
                        for (px, c) in rgba.chunks_exact_mut(4).zip(coverage) {
                            over(
                                px,
                                [colour[0], colour[1], colour[2]],
                                (f32::from(c) / 255.0) * opacity * alpha,
                            );
                        }
                    }
                    Some(ramp) => {
                        for (i, (px, c)) in rgba.chunks_exact_mut(4).zip(coverage).enumerate() {
                            if c == 0 {
                                continue;
                            }
                            let (x, y) =
                                ((i % w as usize) as f64 + 0.5, (i / w as usize) as f64 + 0.5);
                            let (colour, alpha) = ramp.at(x, y);
                            over(px, colour, (f32::from(c) / 255.0) * opacity * alpha);
                        }
                    }
                }
            }

            if let (Some(stroke), true) = (item.stroke, item.stroke_width > 0.0) {
                // A stroke is a brush run along the path, which is exactly what the
                // paint rasteriser already does — one widened-path implementation
                // for both, rather than two that can disagree (K-237).
                // The outline follows the trimmed piece, and it is drawn **open**
                // whatever the path was: a trim is what turns a closed ring into a
                // stroke with two ends.
                let points: Vec<(f64, f64)> = match &trimmed {
                    Some(_) => placed.vertices.iter().map(|v| v.pos).collect(),
                    None => flatten_path(&placed),
                };
                // A copy is a scaled **drawing**, not a scaled path: its outline
                // and its dashes grow with it, or a copy at half size would be a
                // shape with an outline twice as heavy.
                let scale = copy.scale();
                // Dashes cut the outline into pieces, each of which is a brush run
                // of its own (K-552). A solid outline is one piece, which is the
                // same single run it always was.
                let pattern: Vec<f64> = item.dash_pattern_at(t).iter().map(|d| d * scale).collect();
                let pieces = if pattern.is_empty() {
                    vec![points]
                } else {
                    dashed(&points, &pattern, item.dash_offset.value_at(t) * scale)
                };
                let brushes: Vec<crate::paint::PaintStroke> = pieces
                    .into_iter()
                    .filter(|p| p.len() >= 2)
                    .map(|points| crate::paint::PaintStroke {
                        id: item.id,
                        name: item.name.clone(),
                        points,
                        // A vector outline is not a gesture: there is no stylus
                        // behind it and its width is the item's own (K-583).
                        pressures: Vec::new(),
                        colour: stroke,
                        width: item.stroke_width * scale,
                        // A vector outline has a hard edge; the rasteriser keeps
                        // half a pixel of falloff whatever this says, which is the
                        // anti-aliasing rather than a soft brush.
                        hardness: 1.0,
                        // A vector outline is round-capped and round-joined, which
                        // is what the round brush already draws.
                        shape: crate::paint::BrushShape::Round,
                        // The item's own opacity, faded by this copy's share of the
                        // repeater's ramp (K-553) — the same number the fill above
                        // multiplied its coverage by, in the per cent the brush
                        // reads it in.
                        opacity: item.opacity * ramp,
                        // A vector outline is drawn whole; a shape item's own trim
                        // paths are a shape-layer feature, not the brush's (K-549),
                        // so the whole path every time and no clock to read.
                        start: crate::anim::Property::zero(),
                        end: crate::anim::Property::fixed(100.0),
                        mode: crate::paint::PaintMode::Paint,
                        // A shape item's outline lays its colour down; a blend of
                        // its own would be a shape-layer feature (K-550).
                        blend: crate::model::BlendMode::Normal,
                        clone_offset: (0.0, 0.0),
                        extra: serde_json::Map::new(),
                    })
                    .collect();
                crate::paint::apply_strokes(
                    &mut rgba,
                    w,
                    h,
                    max_x - min_x,
                    max_y - min_y,
                    &brushes,
                    0.0,
                );
            }
        }
    }
    rgba
}

/// How many straight steps each cubic segment becomes when a path is walked for
/// stroking.
///
/// Sixteen is smooth to well past the sizes a stroke is drawn at, and a fixed
/// count keeps the result identical on every machine (docs/14 §determinism).
const FLATTEN_STEPS: usize = 16;

/// The path as a polyline: each cubic segment walked in [`FLATTEN_STEPS`] steps.
/// A closed path returns to its first point, so its outline closes.
pub fn flatten_path(path: &BezierPath) -> Vec<(f64, f64)> {
    let n = path.vertices.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![path.vertices[0].pos];
    }
    let mut out = vec![path.vertices[0].pos];
    let last = if path.closed { n } else { n - 1 };
    for i in 0..last {
        let a = &path.vertices[i];
        let b = &path.vertices[(i + 1) % n];
        let p0 = a.pos;
        let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
        let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
        let p3 = b.pos;
        for step in 1..=FLATTEN_STEPS {
            let t = step as f64 / FLATTEN_STEPS as f64;
            out.push(cubic_at(p0, p1, p2, p3, t));
        }
    }
    out
}

fn cubic_at(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// A polyline as a [`BezierPath`]: every handle zero, so the segments stay
/// straight and the one rasteriser in the crate can draw it.
fn polyline_path(points: &[(f64, f64)], closed: bool) -> BezierPath {
    BezierPath {
        vertices: points
            .iter()
            .map(|&pos| Vertex {
                pos,
                tan_in: (0.0, 0.0),
                tan_out: (0.0, 0.0),
            })
            .collect(),
        closed,
    }
}

/// `points` re-started `shift` per cent of its own length along, wrapping — the
/// same ring of points with the seam moved.
///
/// Only meaningful for a closed polyline (one whose last point is its first),
/// which is what a closed path flattens to. A shift of zero, or a path with no
/// length to measure, hands the points straight back.
fn rotated(points: &[(f64, f64)], shift: f64) -> Vec<(f64, f64)> {
    let shift = shift.rem_euclid(100.0);
    if shift == 0.0 || points.len() < 2 {
        return points.to_vec();
    }
    // Walk to the point `shift` per cent along, cut there, and hand back the
    // two pieces the other way round. `trimmed` already knows how to cut a
    // polyline by length, so the two cuts are the two halves of the ring.
    let head = crate::paint::trimmed(points, 0.0, shift);
    let tail = crate::paint::trimmed(points, shift, 100.0);
    if head.len() < 2 || tail.len() < 2 {
        return points.to_vec();
    }
    let mut out = tail;
    // The join is one shared point, not two: the tail ends where the head
    // begins.
    out.extend_from_slice(&head[1..]);
    out
}

/// A gradient fill worked out for one drawn copy: the ramp's geometry in the
/// **buffer's** own pixels, and the colours it runs through as a lookup table.
///
/// A table rather than a lerp per pixel because the two ends are scene-linear
/// and the buffer is 8-bit: mixing has to happen in linear and be encoded
/// after, which is a transcendental per pixel done honestly and a table lookup
/// done sensibly. 256 steps is finer than the 8-bit result can show.
struct Ramp {
    radial: bool,
    start: (f64, f64),
    axis: (f64, f64),
    inv_len2: f64,
    inv_len: f64,
    lut: [[u8; 4]; RAMP_STEPS],
}

/// How many colours a gradient is worked out at. One per 8-bit level, so the
/// table cannot be the thing you can see.
const RAMP_STEPS: usize = 256;

impl Ramp {
    /// The colour at a pixel centre, `[r, g, b]` encoded and the alpha 0..1.
    fn at(&self, x: f64, y: f64) -> ([u8; 3], f32) {
        let (dx, dy) = (x - self.start.0, y - self.start.1);
        // The Gradient effect's two readings (docs/08 §3.35): linear projects
        // onto the axis, radial measures how far out you are with the end
        // point on the outer edge. Both reciprocals were floored against one
        // epsilon when the ramp was built, so a zero-length axis is one flat
        // colour rather than a division by zero (docs/14 §4).
        let along = if self.radial {
            (dx * dx + dy * dy).sqrt() * self.inv_len
        } else {
            (dx * self.axis.0 + dy * self.axis.1) * self.inv_len2
        };
        let i = (along.clamp(0.0, 1.0) * (RAMP_STEPS - 1) as f64).round() as usize;
        let c = self.lut[i.min(RAMP_STEPS - 1)];
        ([c[0], c[1], c[2]], f32::from(c[3]) / 255.0)
    }
}

/// How many straight steps a round join is drawn in, per quarter turn.
///
/// Four is smooth at the sizes an offset outline is drawn at, and a count taken
/// from the angle rather than the size keeps the answer identical on every
/// machine (docs/14 §determinism).
const JOIN_STEPS_PER_QUARTER: f64 = 4.0;

/// `points` pushed `amount` layer pixels **out** of itself, corners rounded —
/// After Effects' Offset Paths, with the one join this crate draws (K-554).
///
/// "Out" is decided by the ring's own winding for a closed path, so a positive
/// amount always grows the shape whichever way round its points were written.
/// An open path has no inside, so it is simply moved to the side its own
/// direction puts on the left.
///
/// **Self-intersections are left in.** Offsetting inwards by more than a curve
/// bends can fold the outline back through itself; the non-zero winding fill
/// swallows most of what that produces, and unpicking it properly is a
/// polygon-clipping library rather than thirty lines. The failure is visible
/// and local, which is why it is a limit rather than a trap.
fn offset_polyline(points: &[(f64, f64)], amount: f64, closed: bool) -> Vec<(f64, f64)> {
    // A closed path flattens with its first point repeated at the end; the
    // offset works on the ring, and closes itself again at the end.
    let ring: &[(f64, f64)] = if closed && points.len() > 2 && points[0] == points[points.len() - 1]
    {
        &points[..points.len() - 1]
    } else {
        points
    };
    if ring.len() < 2 {
        return points.to_vec();
    }

    // Which side is "out". The shoelace area is positive for the winding a
    // shape tool draws, and a path written the other way round gets the sign
    // flipped so that a positive amount still grows it.
    let sign = if closed {
        let mut area = 0.0;
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            area += a.0 * b.1 - b.0 * a.1;
        }
        if area < 0.0 {
            -1.0
        } else {
            1.0
        }
    } else {
        1.0
    };
    let d = amount * sign;

    // The normal of the segment starting at `i`, or `None` where the segment
    // has no length to take a direction from.
    let normal = |i: usize| -> Option<(f64, f64)> {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        (len > 1e-12).then(|| (dy / len, -dx / len))
    };

    let last = if closed { ring.len() } else { ring.len() - 1 };
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(ring.len() * 2);
    for i in 0..ring.len() {
        // The segments that meet at this point. An open path's two ends have
        // only one each, which is what moves an open path to the side.
        let before = (closed || i > 0)
            .then(|| normal((i + ring.len() - 1) % ring.len()))
            .flatten();
        let after = (i < last).then(|| normal(i)).flatten();
        let p = ring[i];
        match (before, after) {
            (Some(n0), Some(n1)) => {
                out.push((p.0 + d * n0.0, p.1 + d * n0.1));
                // The corner opens on the side the offset is going: that gap
                // is what a round join fills. On the other side the two
                // offset ends overlap, and joining them straight is the whole
                // of the self-intersection this leaves behind.
                let cross = n0.0 * n1.1 - n0.1 * n1.0;
                if cross * d > 0.0 {
                    let angle = (n0.0 * n1.0 + n0.1 * n1.1).clamp(-1.0, 1.0).acos();
                    let steps =
                        (angle / std::f64::consts::FRAC_PI_2 * JOIN_STEPS_PER_QUARTER).ceil();
                    let steps = steps.max(1.0) as usize;
                    for step in 1..steps {
                        let a = angle * (step as f64 / steps as f64) * cross.signum();
                        let (sin, cos) = a.sin_cos();
                        let (nx, ny) = (n0.0 * cos - n0.1 * sin, n0.0 * sin + n0.1 * cos);
                        out.push((p.0 + d * nx, p.1 + d * ny));
                    }
                }
                out.push((p.0 + d * n1.0, p.1 + d * n1.1));
            }
            (Some(n), None) | (None, Some(n)) => out.push((p.0 + d * n.0, p.1 + d * n.1)),
            (None, None) => out.push(p),
        }
    }
    if closed && out.len() > 1 {
        out.push(out[0]);
    }
    out
}

/// A 2-D affine transform, `[a, b, c, d, e, f]`, mapping
/// `(x, y)` to `(a x + c y + e, b x + d y + f)` — the order every 2-D graphics
/// library writes it in.
///
/// Six numbers rather than a matrix type from somewhere: this is the only place
/// in the crate that composes transforms in the *art's* own space, and a
/// dependency for six multiplications would be a dependency for six
/// multiplications.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// A move and nothing else.
    #[must_use]
    pub fn translation(dx: f64, dy: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, dx, dy])
    }

    /// How much bigger this transform draws things: the square root of the area
    /// it multiplies by, which is the one number a *width* can be scaled by
    /// whatever the rotation. A mirrored transform has a negative determinant
    /// and still draws at a size, hence the absolute value.
    #[must_use]
    pub fn scale(&self) -> f64 {
        let m = self.0;
        (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
    }

    /// This transform followed by `next` — read left to right, which is how the
    /// repeater stacks its steps.
    #[must_use]
    pub fn then(self, next: &Affine) -> Affine {
        let (m, n) = (self.0, next.0);
        Affine([
            m[0] * n[0] + m[1] * n[2],
            m[0] * n[1] + m[1] * n[3],
            m[2] * n[0] + m[3] * n[2],
            m[2] * n[1] + m[3] * n[3],
            m[4] * n[0] + m[5] * n[2] + n[4],
            m[4] * n[1] + m[5] * n[3] + n[5],
        ])
    }

    #[must_use]
    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let m = self.0;
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }

    /// The transform that undoes this one, or the identity where it cannot be
    /// undone — a copy scaled to nothing has no way back, and an identity is
    /// the answer that draws something rather than dividing by zero (docs/14).
    #[must_use]
    pub fn inverse(&self) -> Affine {
        let m = self.0;
        let det = m[0] * m[3] - m[1] * m[2];
        if det.abs() < 1e-12 {
            return Affine::IDENTITY;
        }
        let inv = 1.0 / det;
        let (a, b, c, d) = (m[3] * inv, -m[1] * inv, -m[2] * inv, m[0] * inv);
        Affine([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
    }
}

/// One step of the repeater: move by `position`, turn by `rotation` degrees and
/// scale by `scale`, all about `anchor`.
fn repeat_step(position: (f64, f64), rotation: f64, scale: f64, anchor: (f64, f64)) -> Affine {
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (a, b, c, d) = (cos * scale, sin * scale, -sin * scale, cos * scale);
    // Move to the anchor, turn and scale there, move back, then translate.
    Affine([
        a,
        b,
        c,
        d,
        anchor.0 - (a * anchor.0 + c * anchor.1) + position.0,
        anchor.1 - (b * anchor.0 + d * anchor.1) + position.1,
    ])
}

/// The most pieces one dashed outline is cut into.
///
/// A pattern fine enough to need more than this is, at any size a stroke is
/// drawn at, a solid line to the eye — and cutting it into a hundred thousand
/// pieces would cost a frame to draw something indistinguishable from one
/// piece. Past the ceiling the outline is drawn **solid** rather than truncated,
/// because a stroke that stopped half way along would be a visible wrong answer
/// where a solid one is an invisible one.
const MAX_DASHES: usize = 4096;

/// `points` cut into its dashes: the "on" pieces of `pattern` (dash, gap, dash,
/// gap, in the polyline's own units), started `offset` along the path.
///
/// The cutting is the same length-measured cut a trim makes, so a dash and a
/// trim agree about where "ten units along" is.
fn dashed(points: &[(f64, f64)], pattern: &[f64], offset: f64) -> Vec<Vec<(f64, f64)>> {
    let whole = || vec![points.to_vec()];
    let cycle: f64 = pattern.iter().sum();
    if pattern.is_empty() || cycle <= 0.0 || points.len() < 2 {
        return whole();
    }
    let total: f64 = points
        .windows(2)
        .map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt())
        .sum();
    if total <= 0.0 || (total / cycle) * pattern.len() as f64 > MAX_DASHES as f64 {
        return whole();
    }

    let mut out = Vec::new();
    // Start inside the cycle *before* the path, so the first dash can be part
    // way through when the offset says so.
    let mut at = -offset.rem_euclid(cycle);
    let mut i = 0usize;
    while at < total {
        let length = pattern[i % pattern.len()];
        let end = at + length;
        // Even entries are dashes, odd ones gaps — the SVG and AE convention.
        if i.is_multiple_of(2) && length > 0.0 && end > 0.0 && at < total {
            let piece = crate::paint::trimmed(
                points,
                at.max(0.0) / total * 100.0,
                end.min(total) / total * 100.0,
            );
            if piece.len() >= 2 {
                out.push(piece);
            }
        }
        at = end;
        i += 1;
    }
    out
}

/// `path` with `m` applied to it.
///
/// A vertex's **position** is moved; its **handles** are turned and scaled but
/// not moved, because a handle is a direction from its vertex rather than a
/// place — moving it as well would translate the curve twice.
fn transform_path(path: &BezierPath, m: &Affine) -> BezierPath {
    let vector = |(x, y): (f64, f64)| {
        let a = m.0;
        (a[0] * x + a[2] * y, a[1] * x + a[3] * y)
    };
    BezierPath {
        vertices: path
            .vertices
            .iter()
            .map(|v| crate::mask::Vertex {
                pos: m.apply(v.pos),
                tan_in: vector(v.tan_in),
                tan_out: vector(v.tan_out),
            })
            .collect(),
        closed: path.closed,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::mask::Mask;

    fn square(x: f64, y: f64, side: f64) -> BezierPath {
        Mask::rectangle(x, y, side, side).path
    }

    fn item(path: BezierPath) -> ShapeItem {
        ShapeItem::filled("Rectangle", path, LinearColour([1.0, 0.0, 0.0, 1.0]))
    }

    fn alpha_at(rgba: &[u8], w: u32, x: u32, y: u32) -> u8 {
        rgba[((y * w + x) as usize) * 4 + 3]
    }

    fn rgb_at(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) as usize) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }

    #[test]
    fn a_layers_size_is_the_box_its_art_fills() {
        let contents = vec![item(square(10.0, 20.0, 30.0))];
        assert_eq!(
            contents_bounds(&contents, 0.0),
            Some((10.0, 20.0, 40.0, 50.0))
        );

        // Two items: the box holds both.
        let contents = vec![item(square(10.0, 20.0, 30.0)), item(square(-5.0, 0.0, 8.0))];
        assert_eq!(
            contents_bounds(&contents, 0.0),
            Some((-5.0, 0.0, 40.0, 50.0))
        );

        assert!(contents_bounds(&[], 0.0).is_none());
    }

    #[test]
    fn a_stroke_widens_the_box_by_half_its_width() {
        let mut it = item(square(0.0, 0.0, 10.0));
        it.stroke = Some(LinearColour::BLACK);
        it.stroke_width = 4.0;
        assert_eq!(it.bounds(0.0), Some((-2.0, -2.0, 12.0, 12.0)));
    }

    #[test]
    fn the_box_holds_the_whole_curve_not_just_its_ends() {
        // One vertex with a long handle: the control point is outside the
        // straight line between the vertices, and the box has to hold it.
        let mut path = square(0.0, 0.0, 10.0);
        path.vertices[0].tan_out = (0.0, -20.0);
        let bounds = item(path).bounds(0.0).expect("bounds");
        assert!(
            bounds.1 <= -20.0,
            "the handle reaches above the box: {bounds:?}"
        );
    }

    #[test]
    fn a_filled_shape_draws_its_colour_inside_and_nothing_outside() {
        let contents = vec![item(square(0.0, 0.0, 20.0))];
        // Rasterised into its own bounding box at 1:1.
        let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(alpha_at(&rgba, 20, 10, 10), 255, "inside the square");
        assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0]);

        // A smaller square in a bigger box leaves the rest transparent.
        let contents = vec![item(square(5.0, 5.0, 5.0))];
        let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(alpha_at(&rgba, 20, 7, 7), 255);
        assert_eq!(alpha_at(&rgba, 20, 18, 18), 0, "outside the art");
    }

    /// The whole reason a shape layer is vector: the same art at twice the
    /// resolution is the same picture, twice as big.
    #[test]
    fn a_shape_is_drawn_at_whatever_resolution_it_is_asked_for() {
        let contents = vec![item(square(0.0, 0.0, 10.0))];
        let small = rasterise_contents(&contents, 10, 10, 0.0, 0.0, 10.0, 10.0, 0.0);
        let big = rasterise_contents(&contents, 40, 40, 0.0, 0.0, 10.0, 10.0, 0.0);
        assert_eq!(alpha_at(&small, 10, 5, 5), 255);
        assert_eq!(alpha_at(&big, 40, 20, 20), 255);
        assert_eq!(big.len(), small.len() * 16);
    }

    #[test]
    fn an_outline_is_drawn_round_the_path() {
        let mut it = item(square(4.0, 4.0, 12.0));
        it.fill = None;
        it.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        it.stroke_width = 3.0;
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

        assert!(alpha_at(&rgba, 20, 4, 10) > 200, "on the left edge");
        assert_eq!(
            alpha_at(&rgba, 20, 10, 10),
            0,
            "and nothing in the middle: this item has no fill"
        );
    }

    #[test]
    fn a_fill_and_a_stroke_are_both_drawn_and_the_stroke_is_on_top() {
        let mut it = item(square(4.0, 4.0, 12.0));
        it.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        it.stroke_width = 3.0;
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

        assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0], "the fill inside");
        assert_eq!(
            rgb_at(&rgba, 20, 4, 10),
            [0, 255, 0],
            "the stroke on the edge"
        );
    }

    #[test]
    fn opacity_fades_the_item() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.opacity = 50.0;
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        let a = alpha_at(&rgba, 20, 10, 10);
        assert!((120..=136).contains(&a), "half opaque, got {a}");
    }

    #[test]
    fn items_are_drawn_in_order() {
        let under = item(square(0.0, 0.0, 20.0));
        let mut over = item(square(0.0, 0.0, 20.0));
        over.fill = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        let rgba = rasterise_contents(&[under, over], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(
            rgb_at(&rgba, 20, 10, 10),
            [0, 0, 255],
            "the later item wins"
        );
    }

    #[test]
    fn nothing_in_it_draws_nothing_and_never_panics() {
        assert!(rasterise_contents(&[], 4, 4, 0.0, 0.0, 4.0, 4.0, 0.0)
            .iter()
            .all(|&b| b == 0));
        assert!(rasterise_contents(&[], 0, 0, 0.0, 0.0, 1.0, 1.0, 0.0).is_empty());

        // A path with no vertices, and one with a single vertex.
        let empty = item(BezierPath {
            vertices: Vec::new(),
            closed: true,
        });
        assert!(empty.bounds(0.0).is_none());
        assert!(rasterise_contents(&[empty], 4, 4, 0.0, 0.0, 4.0, 4.0, 0.0)
            .iter()
            .all(|&b| b == 0));
    }

    #[test]
    fn flattening_walks_the_curve_and_closes_a_closed_path() {
        let path = square(0.0, 0.0, 10.0);
        let points = flatten_path(&path);
        // Four segments, sixteen steps each, plus the first point.
        assert_eq!(points.len(), 4 * FLATTEN_STEPS + 1);
        assert_eq!(points.first(), points.last(), "a closed path comes home");

        let open = BezierPath {
            vertices: path.vertices.clone(),
            closed: false,
        };
        assert_eq!(flatten_path(&open).len(), 3 * FLATTEN_STEPS + 1);
    }

    /// Total length of a polyline, for the trim tests below.
    fn length(points: &[(f64, f64)]) -> f64 {
        points
            .windows(2)
            .map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt())
            .sum()
    }

    fn ink(rgba: &[u8]) -> u32 {
        rgba.chunks_exact(4).map(|p| u32::from(p[3])).sum()
    }

    /// The ink in one vertical band of the picture, for telling two copies of
    /// the same art apart by how much of it there is.
    fn ink_in(rgba: &[u8], w: u32, xs: std::ops::Range<u32>) -> u32 {
        rgba.chunks_exact(4)
            .enumerate()
            .filter(|(i, _)| xs.contains(&(*i as u32 % w)))
            .map(|(_, p)| u32::from(p[3]))
            .sum()
    }

    #[test]
    fn an_untrimmed_shape_is_drawn_from_its_curve_and_not_a_polyline_of_it() {
        let it = item(square(0.0, 0.0, 20.0));
        assert!(!it.trims_at(0.0));
        assert!(it.trimmed_at(0.0).is_none(), "nothing to cut");

        // The whole path asked for explicitly is still the whole path.
        let mut whole = item(square(0.0, 0.0, 20.0));
        whole.trim_start = Property::fixed(0.0);
        whole.trim_end = Property::fixed(100.0);
        assert!(whole.trimmed_at(0.0).is_none());
    }

    #[test]
    fn a_trim_cuts_the_path_by_its_own_length() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.trim_end = Property::fixed(50.0);
        let half = it.trimmed_at(0.0).expect("a trimmed piece");
        let whole = length(&flatten_path(&it.path));
        assert!(
            (length(&half) - whole / 2.0).abs() < 1e-6,
            "half the perimeter, got {} of {whole}",
            length(&half)
        );
        // It starts where the path starts.
        assert_eq!(half.first().copied(), Some(it.path.vertices[0].pos));
    }

    #[test]
    fn a_trimmed_fill_closes_the_piece_that_is_left() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.trim_end = Property::fixed(50.0);
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        // Half a square's perimeter, closed, is a triangle over two corners —
        // it covers less than the whole square and more than nothing.
        let full = rasterise_contents(
            &[item(square(0.0, 0.0, 20.0))],
            20,
            20,
            0.0,
            0.0,
            20.0,
            20.0,
            0.0,
        );
        assert!(ink(&rgba) > 0, "something is left");
        assert!(ink(&rgba) < ink(&full), "and less than the whole square");
    }

    #[test]
    fn an_end_at_or_below_the_start_draws_nothing() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.trim_start = Property::fixed(60.0);
        it.trim_end = Property::fixed(40.0);
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(ink(&rgba), 0, "the first frame of a write-on");
    }

    #[test]
    fn the_offset_slides_the_piece_round_a_closed_path_without_shortening_it() {
        let quarter = |offset: f64| {
            let mut it = item(square(0.0, 0.0, 20.0));
            it.trim_end = Property::fixed(25.0);
            it.trim_offset = Property::fixed(offset);
            it.trimmed_at(0.0).expect("a piece")
        };
        let (a, b) = (quarter(0.0), quarter(90.0));
        assert!(
            (length(&a) - length(&b)).abs() < 1e-6,
            "the same length, slid: {} vs {}",
            length(&a),
            length(&b)
        );
        assert_ne!(a.first(), b.first(), "and it starts somewhere else");
        // 360 degrees is once round: back where it began.
        let round = quarter(360.0);
        assert!((round[0].0 - a[0].0).abs() < 1e-6 && (round[0].1 - a[0].1).abs() < 1e-6);
    }

    #[test]
    fn an_open_paths_offset_runs_the_window_off_the_end_rather_than_wrapping() {
        let mut path = square(0.0, 0.0, 20.0);
        path.closed = false;
        let mut it = item(path);
        it.trim_start = Property::fixed(0.0);
        it.trim_end = Property::fixed(50.0);
        it.trim_offset = Property::fixed(360.0); // a whole path's worth
        let piece = it.trimmed_at(0.0).expect("a piece");
        assert!(piece.len() < 2, "slid clean off: {piece:?}");
    }

    #[test]
    fn a_keyed_trim_is_read_on_the_layers_clock() {
        let mut it = item(square(0.0, 0.0, 20.0));
        let key = |secs: i64, value: f64| crate::anim::Keyframe {
            time: crate::time::Rational::new(secs, 1).expect("a whole second"),
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        it.trim_end.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 100.0)]);
        let at = |t: f64| {
            ink(&rasterise_contents(
                &[it.clone()],
                20,
                20,
                0.0,
                0.0,
                20.0,
                20.0,
                t,
            ))
        };
        assert_eq!(at(0.0), 0, "nothing drawn yet");
        assert!(at(1.0) > at(0.5), "and it fills in as it plays");
    }

    #[test]
    fn an_untrimmed_item_is_absent_from_the_file() {
        let json = serde_json::to_string(&item(square(0.0, 0.0, 4.0))).expect("json");
        assert!(!json.contains("trim"), "nothing about a trim: {json}");
        let back: ShapeItem = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back.trim_end.value_at(0.0), 100.0, "the default comes back");
    }

    fn outlined(item: &ShapeItem, t: f64) -> Vec<u8> {
        rasterise_contents(std::slice::from_ref(item), 40, 40, 0.0, 0.0, 40.0, 40.0, t)
    }

    /// A dashed outline is on, off, on, off along its own length — so it puts
    /// down less ink than the solid one and more than none.
    #[test]
    fn a_dashed_outline_leaves_gaps_in_itself() {
        let mut solid = item(square(5.0, 5.0, 30.0));
        solid.fill = None;
        solid.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        solid.stroke_width = 2.0;

        let mut dashed_item = solid.clone();
        dashed_item.dashes = vec![Property::fixed(6.0), Property::fixed(6.0)];

        let (a, b) = (
            ink(&outlined(&solid, 0.0)),
            ink(&outlined(&dashed_item, 0.0)),
        );
        assert!(b > 0, "something is drawn");
        assert!(b < a, "and less of it than solid: {b} of {a}");
    }

    #[test]
    fn a_dash_of_nothing_is_a_solid_outline() {
        let mut it = item(square(5.0, 5.0, 30.0));
        it.fill = None;
        it.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        it.stroke_width = 2.0;
        let solid = outlined(&it, 0.0);

        // An empty list, and a list of zeros, are both "no dashes".
        assert!(it.dash_pattern_at(0.0).is_empty());
        it.dashes = vec![Property::zero(), Property::zero()];
        assert!(it.dash_pattern_at(0.0).is_empty());
        assert_eq!(outlined(&it, 0.0), solid, "byte for byte the solid one");
    }

    #[test]
    fn an_odd_dash_list_repeats_itself() {
        let mut it = item(square(0.0, 0.0, 10.0));
        it.dashes = vec![Property::fixed(4.0)];
        assert_eq!(it.dash_pattern_at(0.0), vec![4.0, 4.0], "four on, four off");
        it.dashes = vec![
            Property::fixed(4.0),
            Property::fixed(2.0),
            Property::fixed(1.0),
        ];
        assert_eq!(it.dash_pattern_at(0.0), vec![4.0, 2.0, 1.0, 4.0, 2.0, 1.0]);
    }

    #[test]
    fn the_dashes_are_cut_by_length_and_the_offset_slides_them() {
        // A straight line 100 long: 10 on, 10 off gives five dashes of ten.
        let line: Vec<(f64, f64)> = vec![(0.0, 0.0), (100.0, 0.0)];
        let pieces = dashed(&line, &[10.0, 10.0], 0.0);
        assert_eq!(pieces.len(), 5);
        assert_eq!(pieces[0], vec![(0.0, 0.0), (10.0, 0.0)]);
        assert_eq!(pieces[1], vec![(20.0, 0.0), (30.0, 0.0)]);

        // An offset of five slides the pattern back along the path, so the
        // first dash is the tail of one that began before the line did.
        let slid = dashed(&line, &[10.0, 10.0], 5.0);
        assert_eq!(slid[0], vec![(0.0, 0.0), (5.0, 0.0)]);
        assert_eq!(slid[1], vec![(15.0, 0.0), (25.0, 0.0)]);

        // A whole cycle of offset is no offset at all.
        assert_eq!(dashed(&line, &[10.0, 10.0], 20.0), pieces);
    }

    #[test]
    fn a_pattern_too_fine_to_see_is_drawn_solid_rather_than_cut_to_pieces() {
        let line: Vec<(f64, f64)> = vec![(0.0, 0.0), (1_000_000.0, 0.0)];
        let pieces = dashed(&line, &[1.0, 1.0], 0.0);
        assert_eq!(pieces.len(), 1, "one solid piece, not half a million");
        assert_eq!(pieces[0], line);
    }

    #[test]
    fn a_keyed_dash_is_read_on_the_layers_clock() {
        let mut it = item(square(5.0, 5.0, 30.0));
        it.fill = None;
        it.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        it.stroke_width = 2.0;
        let key = |secs: i64, value: f64| crate::anim::Keyframe {
            time: crate::time::Rational::new(secs, 1).expect("a whole second"),
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        let mut gap = Property::fixed(0.0);
        gap.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 20.0)]);
        it.dashes = vec![Property::fixed(6.0), gap];
        assert!(
            ink(&outlined(&it, 1.0)) < ink(&outlined(&it, 0.0)),
            "a gap that opens takes ink away as it plays"
        );
    }

    #[test]
    fn an_undashed_item_is_absent_from_the_file() {
        let json = serde_json::to_string(&item(square(0.0, 0.0, 4.0))).expect("json");
        assert!(!json.contains("dash"), "nothing about dashes: {json}");
        let mut it = item(square(0.0, 0.0, 4.0));
        it.dashes = vec![Property::fixed(6.0), Property::fixed(3.0)];
        let json = serde_json::to_string(&it).expect("json");
        assert!(
            json.contains("[6.0,3.0]"),
            "bare numbers while still: {json}"
        );
        let back: ShapeItem = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back.dashes.len(), 2);
        assert_eq!(back.dashes[1].value_at(0.0), 3.0);
    }

    /// A linear ramp runs from the fill at one point to the gradient colour at
    /// the other, and everything between is a mix of the two.
    #[test]
    fn a_linear_gradient_ramps_across_the_fill() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.fill = Some(LinearColour([1.0, 0.0, 0.0, 1.0]));
        it.gradient = 1;
        it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        it.gradient_start_x = Property::fixed(0.0);
        it.gradient_end_x = Property::fixed(20.0);
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

        let left = rgb_at(&rgba, 20, 0, 10);
        let right = rgb_at(&rgba, 20, 19, 10);
        let middle = rgb_at(&rgba, 20, 10, 10);
        assert!(
            left[0] > 200 && left[2] < 60,
            "the fill at the start: {left:?}"
        );
        assert!(
            right[2] > 200 && right[0] < 60,
            "the gradient colour at the end: {right:?}"
        );
        assert!(
            middle[0] > 40 && middle[2] > 40,
            "and a mix of the two between: {middle:?}"
        );
        // The ramp runs *across* the shape, not down it.
        assert_eq!(
            rgb_at(&rgba, 20, 10, 2),
            middle,
            "no ramp along the other axis"
        );
    }

    /// Radial measures how far *out* you are, the end point sitting on the
    /// outer edge — so the middle is the fill and every direction ramps away.
    #[test]
    fn a_radial_gradient_ramps_out_from_its_start() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.fill = Some(LinearColour([1.0, 0.0, 0.0, 1.0]));
        it.gradient = 2;
        it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        it.gradient_start_x = Property::fixed(10.0);
        it.gradient_start_y = Property::fixed(10.0);
        it.gradient_end_x = Property::fixed(20.0);
        it.gradient_end_y = Property::fixed(10.0);
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

        assert!(rgb_at(&rgba, 20, 10, 10)[0] > 200, "the fill in the middle");
        // Every direction ramps the same way, which is what makes it radial.
        let out = [(0u32, 10u32), (19, 10), (10, 0), (10, 19)];
        for (x, y) in out {
            assert!(
                rgb_at(&rgba, 20, x, y)[2] > 150,
                "the far colour at the edge ({x}, {y})"
            );
        }
    }

    /// The gradient belongs to the art, so a repeated copy carries it (K-553).
    #[test]
    fn a_repeated_copy_carries_its_gradient_with_it() {
        let mut it = item(square(0.0, 0.0, 10.0));
        it.fill = Some(LinearColour([1.0, 0.0, 0.0, 1.0]));
        it.gradient = 1;
        it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        it.gradient_end_x = Property::fixed(10.0);
        it.repeat_copies = Property::fixed(2.0);
        it.repeat_position_x = Property::fixed(10.0);
        let rgba = rasterise_contents(&[it], 20, 10, 0.0, 0.0, 20.0, 10.0, 0.0);
        assert!(
            rgb_at(&rgba, 20, 0, 5)[0] > 200,
            "the first copy starts red"
        );
        assert!(
            rgb_at(&rgba, 20, 10, 5)[0] > 200,
            "and so does the copy, ten along: the ramp moved with it"
        );
    }

    /// A flat fill is what every shape has until somebody ramps it, and it
    /// draws exactly the pixels it drew before there were gradients.
    #[test]
    fn a_flat_fill_is_untouched_by_the_gradient_machinery() {
        let plain = item(square(2.0, 3.0, 9.0));
        let mut off = item(square(2.0, 3.0, 9.0));
        off.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        off.gradient_end_x = Property::fixed(9.0);
        assert!(off.ramp_at(0.0, &Affine::IDENTITY, 1.0, 1.0).is_none());
        assert_eq!(
            rasterise_contents(&[plain], 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0),
            rasterise_contents(&[off], 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0),
        );
    }

    /// Both points in the same place is no axis at all: one flat colour rather
    /// than a division by zero (docs/14 §4).
    #[test]
    fn a_gradient_with_no_axis_draws_one_flat_colour_and_never_panics() {
        let mut it = item(square(0.0, 0.0, 10.0));
        it.fill = Some(LinearColour([1.0, 0.0, 0.0, 1.0]));
        it.gradient = 1;
        it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        let rgba = rasterise_contents(&[it.clone()], 10, 10, 0.0, 0.0, 10.0, 10.0, 0.0);
        assert_eq!(rgb_at(&rgba, 10, 2, 2), rgb_at(&rgba, 10, 8, 8));
        it.gradient = 2;
        let _ = rasterise_contents(&[it], 10, 10, 0.0, 0.0, 10.0, 10.0, 0.0);
    }

    #[test]
    fn a_keyed_gradient_point_is_read_on_the_layers_clock() {
        let mut it = item(square(0.0, 0.0, 20.0));
        it.fill = Some(LinearColour([1.0, 0.0, 0.0, 1.0]));
        it.gradient = 1;
        it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        let key = |secs: i64, value: f64| crate::anim::Keyframe {
            time: crate::time::Rational::new(secs, 1).expect("a whole second"),
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        let mut end = Property::fixed(0.0);
        end.animation = crate::anim::Animation::Keyframed(vec![key(0, 4.0), key(1, 40.0)]);
        it.gradient_end_x = end;
        let blue = |t: f64| {
            rgb_at(
                &rasterise_contents(&[it.clone()], 20, 20, 0.0, 0.0, 20.0, 20.0, t),
                20,
                10,
                10,
            )[2]
        };
        assert!(
            blue(0.0) > blue(1.0),
            "a ramp stretched out is less far along in the middle"
        );
    }

    #[test]
    fn a_flat_filled_item_is_absent_from_the_file() {
        let json = serde_json::to_string(&item(square(0.0, 0.0, 4.0))).expect("json");
        assert!(!json.contains("gradient"), "nothing about ramps: {json}");
        let mut it = item(square(0.0, 0.0, 4.0));
        it.gradient = 2;
        it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
        it.gradient_end_x = Property::fixed(4.0);
        let back: ShapeItem =
            serde_json::from_str(&serde_json::to_string(&it).expect("json")).expect("round trip");
        assert_eq!(back.gradient, 2);
        assert_eq!(
            back.gradient_colour,
            Some(LinearColour([0.0, 0.0, 1.0, 1.0]))
        );
        assert_eq!(back.gradient_end_x.value_at(0.0), 4.0);
    }

    /// The outline pushed out of the path: art where the path is not, filled
    /// as one piece.
    #[test]
    fn an_offset_path_grows_the_shape_and_a_negative_one_shrinks_it() {
        let mut it = item(square(5.0, 5.0, 10.0));
        it.offset_amount = Property::fixed(3.0);
        let grown = rasterise_contents(&[it.clone()], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(
            alpha_at(&grown, 20, 3, 10),
            255,
            "outside the path, inside the offset"
        );
        assert_eq!(alpha_at(&grown, 20, 0, 10), 0, "and not beyond it");

        it.offset_amount = Property::fixed(-3.0);
        let shrunk = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(
            alpha_at(&shrunk, 20, 6, 10),
            0,
            "inside the path, outside the offset"
        );
        assert_eq!(
            alpha_at(&shrunk, 20, 10, 10),
            255,
            "and the middle is still there"
        );
    }

    /// The corner of a grown square is a quarter circle, not a square corner:
    /// round is the one join this crate draws.
    #[test]
    fn an_offset_corner_is_rounded_rather_than_mitred() {
        let mut it = item(square(6.0, 6.0, 8.0));
        it.offset_amount = Property::fixed(4.0);
        let rgba = rasterise_contents(&[it], 24, 24, 0.0, 0.0, 24.0, 24.0, 0.0);
        // Straight out from the top edge, right to the offset's edge, is
        // inside. The pixel a mitred corner would reach — the corner of the
        // grown box — is nearly five away from the path's own corner, so a
        // round join of four leaves it outside.
        assert_eq!(alpha_at(&rgba, 24, 10, 2), 255, "square out from the edge");
        assert_eq!(alpha_at(&rgba, 24, 2, 2), 0, "and the corner is cut round");
    }

    /// A path written the other way round is the same shape, so a positive
    /// offset has to grow it either way.
    #[test]
    fn an_offset_grows_a_path_written_either_way_round() {
        let mut backwards = square(5.0, 5.0, 10.0);
        backwards.vertices.reverse();
        let mut it = item(backwards);
        it.offset_amount = Property::fixed(3.0);
        let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
        assert_eq!(alpha_at(&rgba, 20, 3, 10), 255, "grown, not shrunk");
    }

    #[test]
    fn the_box_holds_the_grown_outline() {
        let mut it = item(square(5.0, 5.0, 10.0));
        assert_eq!(it.bounds(0.0), Some((5.0, 5.0, 15.0, 15.0)));
        it.offset_amount = Property::fixed(3.0);
        assert_eq!(it.bounds(0.0), Some((2.0, 2.0, 18.0, 18.0)));
        // Pulled in, the art never needs more room than the path did.
        it.offset_amount = Property::fixed(-3.0);
        assert_eq!(it.bounds(0.0), Some((5.0, 5.0, 15.0, 15.0)));
    }

    /// The identity case: an item nobody has offset draws from its bezier,
    /// exactly as it did before there was an offset at all.
    #[test]
    fn an_offset_of_nothing_is_the_path_itself() {
        let plain = item(square(2.0, 3.0, 9.0));
        let mut zero = item(square(2.0, 3.0, 9.0));
        zero.offset_amount = Property::fixed(0.0);
        assert!(zero.trimmed_at(0.0).is_none(), "nothing to reshape");
        assert_eq!(
            rasterise_contents(&[plain], 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0),
            rasterise_contents(&[zero], 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0),
        );
    }

    /// Offset first, then trim: the trim cuts whatever outline the offset made,
    /// which is longer than the path it came from.
    #[test]
    fn the_trim_cuts_the_offset_outline_and_not_the_path() {
        let mut it = item(square(5.0, 5.0, 10.0));
        it.offset_amount = Property::fixed(3.0);
        it.trim_end = Property::fixed(50.0);
        let piece = it.trimmed_at(0.0).expect("a piece");
        let whole = offset_polyline(&flatten_path(&it.path), 3.0, true);
        let length = |p: &[(f64, f64)]| -> f64 {
            p.windows(2)
                .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
                .sum()
        };
        let (cut, all) = (length(&piece), length(&whole));
        assert!(
            (cut - all / 2.0).abs() < all * 0.01,
            "half of the grown outline: {cut} of {all}"
        );
    }

    #[test]
    fn a_keyed_offset_is_read_on_the_layers_clock() {
        let mut it = item(square(5.0, 5.0, 10.0));
        let key = |secs: i64, value: f64| crate::anim::Keyframe {
            time: crate::time::Rational::new(secs, 1).expect("a whole second"),
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        let mut grow = Property::fixed(0.0);
        grow.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 4.0)]);
        it.offset_amount = grow;
        let at = |t: f64| {
            ink(&rasterise_contents(
                &[it.clone()],
                24,
                24,
                0.0,
                0.0,
                24.0,
                24.0,
                t,
            ))
        };
        assert!(at(1.0) > at(0.0), "the shape swells as it plays");
    }

    #[test]
    fn an_unoffset_item_is_absent_from_the_file() {
        let json = serde_json::to_string(&item(square(0.0, 0.0, 4.0))).expect("json");
        assert!(
            !json.contains("offset_amount"),
            "nothing about offsets: {json}"
        );
        let mut it = item(square(0.0, 0.0, 4.0));
        it.offset_amount = Property::fixed(2.5);
        let back: ShapeItem =
            serde_json::from_str(&serde_json::to_string(&it).expect("json")).expect("round trip");
        assert_eq!(back.offset_amount.value_at(0.0), 2.5);
    }

    /// A repeated square is drawn where the step puts it, and nowhere else.
    #[test]
    fn a_repeater_draws_a_copy_at_every_step() {
        let mut it = item(square(0.0, 0.0, 6.0));
        it.repeat_copies = Property::fixed(3.0);
        it.repeat_position_x = Property::fixed(10.0);
        let contents = vec![it];
        // The box has grown to hold all three, so it is asked for explicitly.
        let rgba = rasterise_contents(&contents, 40, 10, 0.0, 0.0, 40.0, 10.0, 0.0);
        for x in [3, 13, 23] {
            assert_eq!(alpha_at(&rgba, 40, x, 3), 255, "a copy at {x}");
        }
        for x in [8, 18, 33] {
            assert_eq!(alpha_at(&rgba, 40, x, 3), 0, "the gap at {x}");
        }
    }

    /// The layer has to be big enough to hold what the repeater made, or the
    /// copies would be drawn off the edge of their own raster.
    #[test]
    fn the_box_grows_to_hold_the_copies() {
        let mut it = item(square(0.0, 0.0, 6.0));
        assert_eq!(it.bounds(0.0), Some((0.0, 0.0, 6.0, 6.0)));
        it.repeat_copies = Property::fixed(3.0);
        it.repeat_position_x = Property::fixed(10.0);
        assert_eq!(it.bounds(0.0), Some((0.0, 0.0, 26.0, 6.0)));

        // Behind the original as well, when the offset says so.
        it.repeat_offset = Property::fixed(-1.0);
        assert_eq!(it.bounds(0.0), Some((-10.0, 0.0, 16.0, 6.0)));
    }

    /// The identity case, which is every shape until somebody repeats one: the
    /// pixels are the ones drawn before there was a repeater at all.
    #[test]
    fn one_copy_is_no_repeater_at_all() {
        let plain = item(square(2.0, 3.0, 9.0));
        let mut one_copy = item(square(2.0, 3.0, 9.0));
        one_copy.repeat_copies = Property::fixed(1.0);
        one_copy.repeat_position_x = Property::fixed(40.0);
        one_copy.repeat_rotation = Property::fixed(30.0);
        assert_eq!(plain.bounds(0.0), one_copy.bounds(0.0));
        assert_eq!(
            rasterise_contents(&[plain], 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0),
            rasterise_contents(&[one_copy], 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0),
            "one copy draws the very same bytes"
        );
    }

    /// Start and end opacity ramp across the copies drawn — the first at one
    /// end of the ramp, the last at the other.
    #[test]
    fn the_copies_fade_from_the_first_to_the_last() {
        let mut it = item(square(0.0, 0.0, 6.0));
        it.repeat_copies = Property::fixed(3.0);
        it.repeat_position_x = Property::fixed(10.0);
        it.repeat_end_opacity = Property::fixed(0.0);
        let rgba = rasterise_contents(&[it], 40, 10, 0.0, 0.0, 40.0, 10.0, 0.0);
        let (first, middle, last) = (
            alpha_at(&rgba, 40, 3, 3),
            alpha_at(&rgba, 40, 13, 3),
            alpha_at(&rgba, 40, 23, 3),
        );
        assert_eq!(first, 255, "the first copy is the item's own opacity");
        assert!(
            middle > 100 && middle < 200,
            "the middle is half way down the ramp: {middle}"
        );
        assert_eq!(last, 0, "and the last has faded out");
    }

    /// A copy at half size is a *drawing* at half size: its outline thins with
    /// it, or the copy would look like a different shape.
    #[test]
    fn a_scaled_copy_carries_a_scaled_outline() {
        let mut it = item(square(2.0, 2.0, 8.0));
        it.fill = None;
        it.stroke = Some(LinearColour([0.0, 1.0, 0.0, 1.0]));
        it.stroke_width = 4.0;
        it.repeat_copies = Property::fixed(2.0);
        it.repeat_position_x = Property::fixed(20.0);
        it.repeat_scale = Property::fixed(50.0);
        // The second copy is half the size *and* half the outline, so it puts
        // down less than half the first one's ink.
        let rgba = rasterise_contents(&[it], 40, 20, 0.0, 0.0, 40.0, 20.0, 0.0);
        let left: u32 = ink_in(&rgba, 40, 0..20);
        let right: u32 = ink_in(&rgba, 40, 20..40);
        assert!(right > 0, "the copy is drawn");
        assert!(
            right * 2 < left,
            "and it is drawn smaller in both ways: {left} against {right}"
        );
    }

    /// Rotation turns each copy about the anchor, so a step of 90° puts the
    /// fourth copy back where the first one started.
    #[test]
    fn a_rotated_copy_turns_about_the_anchor() {
        let mut it = item(square(8.0, 2.0, 4.0));
        it.repeat_copies = Property::fixed(4.0);
        it.repeat_rotation = Property::fixed(90.0);
        it.repeat_anchor_x = Property::fixed(10.0);
        it.repeat_anchor_y = Property::fixed(10.0);
        let (x0, y0, x1, y1) = it.bounds(0.0).expect("a box");
        // Four quarter turns about (10, 10) put the copies on all four sides of
        // it, so the box is square and centred on the anchor.
        assert!((x1 - x0 - (y1 - y0)).abs() < 1e-9, "a square box");
        assert!(
            ((x0 + x1) / 2.0 - 10.0).abs() < 1e-9 && ((y0 + y1) / 2.0 - 10.0).abs() < 1e-9,
            "centred on the anchor: {x0},{y0} to {x1},{y1}"
        );
    }

    /// A count nobody could draw is held at the ceiling rather than refused —
    /// and a fractional one is a count of things, so it rounds.
    #[test]
    fn the_copy_count_is_held_at_the_ceiling_and_never_fractional() {
        let mut it = item(square(0.0, 0.0, 2.0));
        it.repeat_position_x = Property::fixed(1.0);
        it.repeat_copies = Property::fixed(1e9);
        assert_eq!(it.copies_at(0.0).len(), MAX_COPIES as usize);
        it.repeat_copies = Property::fixed(-4.0);
        assert_eq!(it.copies_at(0.0).len(), 1, "never fewer than the original");
        it.repeat_copies = Property::fixed(2.6);
        assert_eq!(it.copies_at(0.0).len(), 3);
    }

    #[test]
    fn a_keyed_repeater_is_read_on_the_layers_clock() {
        let mut it = item(square(0.0, 0.0, 6.0));
        it.repeat_copies = Property::fixed(3.0);
        let key = |secs: i64, value: f64| crate::anim::Keyframe {
            time: crate::time::Rational::new(secs, 1).expect("a whole second"),
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        let mut step = Property::fixed(0.0);
        step.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 10.0)]);
        it.repeat_position_x = step;
        // Stacked at the head, spread out a second later — and the box knows.
        assert_eq!(it.bounds(0.0), Some((0.0, 0.0, 6.0, 6.0)));
        assert_eq!(it.bounds(1.0), Some((0.0, 0.0, 26.0, 6.0)));
    }

    #[test]
    fn an_unrepeated_item_is_absent_from_the_file() {
        let json = serde_json::to_string(&item(square(0.0, 0.0, 4.0))).expect("json");
        assert!(!json.contains("repeat"), "nothing about copies: {json}");
        let mut it = item(square(0.0, 0.0, 4.0));
        it.repeat_copies = Property::fixed(5.0);
        it.repeat_position_x = Property::fixed(12.0);
        let json = serde_json::to_string(&it).expect("json");
        let back: ShapeItem = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back.repeat_copies.value_at(0.0), 5.0);
        assert_eq!(back.repeat_position_x.value_at(0.0), 12.0);
        assert_eq!(
            back.repeat_scale.value_at(0.0),
            100.0,
            "the default is kept"
        );
    }

    #[test]
    fn drawing_a_shape_is_deterministic() {
        let contents = vec![item(square(2.0, 3.0, 9.0))];
        let once = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0);
        let twice = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0);
        assert_eq!(once, twice);
    }
}
