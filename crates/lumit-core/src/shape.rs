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
//! could draw both from the same geometry from the day they landed.
//!
//! **The layer's own size is the art's bounding box**, and it changes as the art
//! is edited. Every other layer kind has a size fixed by its source; this is the
//! first that does not, and anything caching "how big is this layer" has to
//! follow the document's revision rather than assume (docs/impl/shape-layers.md).
//!
//! **The modifiers are fields on the item, not a tree**. After Effects
//! carries Trim Paths, the Repeater and the rest as entries in a nested group,
//! where their position decides what they act on. Lumit's list is flat, so each
//! modifier is a property of the item it modifies and the order they apply in is
//! fixed and written down here rather than dragged: **combine, then smooth,
//! then twist, then deform, then offset, then trim, then repeat**. A
//! **combine** is the one field that reaches past its own item: it says how this item's path joins the item before it, and the
//! run that makes is drawn once with the first item's paint. The
//! nested tree is still the long-term shape (docs/03 §9.2); nothing stored here
//! stands in its way, because every modifier is absent from the file until it is
//! used.
//!
//! **What is deliberately not here.** Nested groups, wiggle paths, line joins
//! other than round, and animated paths.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::anim::Property;
use crate::mask::BezierPath;
use crate::model::LinearColour;
use crate::pixels::over;

use geometry::{
    arc_of, box_of, control_points, densified, flatten_fine, flatten_path, point_at, ring_of,
    rotated, shoelace, Affine, FINE_STEP, MAX_FINE_POINTS,
};
use raster::{runs, walked, Art};

/// One piece of vector art: a path, and how it is painted.
///
/// A flat list of these per layer rather than After Effects' nested groups —
/// groups are a modifier feature and are later work (docs/impl/shape-layers.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeItem {
    pub id: Uuid,
    pub name: String,
    pub path: BezierPath,
    /// The colour inside the path, a property per channel so it keys. `None`
    /// draws no fill, which is how an outline-only shape is made.
    #[serde(default, with = "colour::keyed")]
    pub fill: Option<[Property; 4]>,
    /// The colour of the outline, and how wide it is in layer pixels. `None`
    /// draws no stroke.
    #[serde(default, with = "colour::keyed")]
    pub stroke: Option<[Property; 4]>,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub stroke_width: Property,
    /// 0..100, like every other opacity in the document.
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub opacity: Property,
    /// **Trim paths**: where along the path the art begins and ends, as
    /// a per cent of the path's own **arc length**, and how far the pair is slid
    /// along it in degrees (360 is once round).
    ///
    /// Per cent of length rather than of vertex count, for the reason a paint
    /// stroke's write-on gives: the eye watches length. The trim cuts
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
    /// **Dashes**: the outline's dash and gap lengths in layer pixels,
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
    /// **A gradient fill**: 0 draws the flat [`fill`](Self::fill), 1
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
    /// How opaque each end of the ramp is, per cent, with everything between
    /// fading evenly from one to the other. Colour and opacity ramp separately
    /// as After Effects does, so a red fading into a clear blue is a half-clear
    /// purple half way along.
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub gradient_start_opacity: Property,
    #[serde(
        default = "crate::paint::full",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_full"
    )]
    pub gradient_end_opacity: Property,
    /// The colours the ramp passes through between its two ends, in any
    /// order, as a stop that animates can pass another. Empty is the plain
    /// two-colour ramp, and is absent from the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gradient_stops: Vec<GradientStop>,
    /// **A morphing path**: the shapes this item's path is keyed to,
    /// in the layer's own time. Empty is the still [`path`](Self::path) above,
    /// which is what every shape is until somebody keys its shape, and is
    /// absent from the file.
    ///
    /// The same [`PathKeyframe`] a mask's shape keys with, and the same
    /// interpolation beneath it: two keys with the same number of points run
    /// point for point, and unequal ones have the sparser path **resampled**
    /// first — cut into as many pieces as the denser one has, by splitting its
    /// own curve, so nothing about the shape moves in the process.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_keys: Vec<crate::mask::PathKeyframe>,
    /// **A boolean combine**: how this item joins the item **before**
    /// it in the layer's list — 0 draws it on its own, 1 unions the two, 2
    /// subtracts this one from that one, 3 keeps only what both cover and 4
    /// keeps only what one of them covers.
    ///
    /// A run of items joined this way is drawn **once**, with the paint and the
    /// modifiers of the *first* item in it: the ones after it lend their path
    /// and nothing else. A combine on the very first item of the list has
    /// nothing before it to join and draws on its own.
    ///
    /// A choice rather than a `Property`, for [`gradient`](Self::gradient)'s
    /// reason: what a combine *is* does not tween, and a number half way
    /// between subtract and intersect would have to mean something.
    #[serde(default, skip_serializing_if = "is_flat")]
    pub combine: u32,
    /// **Offset paths**: how far the outline is pushed **out** of the
    /// path, in layer pixels — negative pulls it in. Zero is the path itself
    /// and is absent from the file.
    ///
    /// The corners are **round**, which is the one join this crate draws. The
    /// offset does not undo its own self-intersections, so an inward offset
    /// past a curve's own radius leaves a small loop that the non-zero winding
    /// fill mostly swallows.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub offset_amount: Property,
    /// **The repeater**: how many copies of the item are drawn, and the
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
    /// **Twist**: the path spun about an axis and seen side-on. `None` is no
    /// twist, which is every shape until somebody adds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twist: Option<Twist>,
    /// **Deform**: the outline moved by a smooth noise field. `None` is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deform: Option<Deform>,
    /// **Smooth**: the whole path rounded off. `None` is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<Smooth>,
    /// **Stroke taper**: the outline thinning towards its ends. `None` is
    /// the full width all the way along.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taper: Option<Taper>,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A twist: the path turned about a line through `centre`, pointing
/// `axis` degrees round from the x axis, and flattened back onto the
/// layer as if seen side-on.
///
/// The turn is not the same everywhere: one end of the shape along the axis
/// stays put and the other turns by `angle`, the way a ribbon twists. So 0
/// leaves the shape alone, a circle pinches to a figure eight at 180, and
/// each further half turn adds another crossing. `rotation` then spins the
/// whole twisted shape about the axis, so a full turn of it loops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Twist {
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub angle: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub rotation: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub axis: Property,
    /// In layer pixels, the art's own coordinates.
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub centre_x: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub centre_y: Property,
}

/// A deform: the outline moved by a smooth noise field laid over the layer.
///
/// The field is read at points along the path, `detail` of them to every
/// `scale` pixels of it, and eased between them, so a curve stays a curve
/// however quickly the field changes: fewer points bend the outline more
/// broadly. `evolution` moves through the field in degrees, a turn a cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deform {
    /// 0 pushes each point along the outline's normal by `amount` pixels; 1
    /// scales its distance from the middle of the path by `amount` per cent.
    #[serde(default, skip_serializing_if = "is_flat")]
    pub mode: u32,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub amount: Property,
    /// The size of the field's features, in layer pixels.
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub scale: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub evolution: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub seed: Property,
    #[serde(default = "four", with = "crate::mask::still_or_keyed")]
    pub detail: Property,
}

/// A smooth: every point of the path averaged with its neighbours either
/// side, `amount` pixels of path each way, so jags and corners round off
/// along the whole of it. An average never leaves the points it was taken
/// over, so the path never bulges past itself; it shrinks a little instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Smooth {
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub amount: Property,
}

/// A stroke taper: the outline narrowing towards its two ends like a brush
/// stroke.
///
/// `start` and `end` are how much of the width is gone at each end, per
/// cent, and `start_length` and `end_length` how far along the line, per cent
/// of it, the narrowing runs. The two lengths are shrunk together when they
/// add up to more than the line, so the ends never cross. `ease` rounds the
/// narrowing from a straight ramp, per cent. `scope` 0 tapers each line on
/// its own; 1 runs one taper along all of them in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Taper {
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub start: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub end: Property,
    #[serde(default = "half", with = "crate::mask::still_or_keyed")]
    pub start_length: Property,
    #[serde(default = "half", with = "crate::mask::still_or_keyed")]
    pub end_length: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub ease: Property,
    #[serde(default, skip_serializing_if = "is_flat")]
    pub scope: u32,
}

fn half() -> Property {
    Property::fixed(50.0)
}

fn four() -> Property {
    Property::fixed(4.0)
}

/// A colour part way along a gradient, between the fill at its start and the
/// gradient colour at its end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// How far along the ramp it sits, per cent. Two stops at one place make
    /// a hard edge.
    #[serde(with = "crate::mask::still_or_keyed")]
    pub position: Property,
    pub colour: LinearColour,
    /// 0..100, like the two ends.
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub opacity: Property,
}

/// The most stops a gradient carries between its two ends. Past what a ramp
/// needs, and it keeps a wild number out of the document.
pub const MAX_GRADIENT_STOPS: usize = 16;

/// serde default for [`ShapeItem::repeat_copies`]: one copy, which is no
/// repeater at all.
fn one() -> Property {
    Property::fixed(1.0)
}

/// True for the zeroth reading of a choice — a flat fill, an item combined with
/// nothing — and so nothing written to the file.
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
            fill: Some(colour::still(fill)),
            stroke: None,
            stroke_width: Property::zero(),
            opacity: crate::paint::full(),
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
            gradient_start_opacity: crate::paint::full(),
            gradient_end_opacity: crate::paint::full(),
            gradient_stops: Vec::new(),
            path_keys: Vec::new(),
            combine: 0,
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
            twist: None,
            deform: None,
            smooth: None,
            taper: None,
            extra: serde_json::Map::new(),
        }
    }

    /// The shape this item has at `t` — its still [`path`](Self::path) where
    /// nothing has keyed it, and the blend of the two keys either side where
    /// something has.
    ///
    /// `t` is the **layer's** clock, the one every other number on the item is
    /// read on.
    pub fn path_at(&self, t: f64) -> std::borrow::Cow<'_, BezierPath> {
        crate::mask::path_at(&self.path_keys, &self.path, t)
    }

    /// Whether this item's shape is keyframed.
    pub fn path_is_animated(&self) -> bool {
        !self.path_keys.is_empty()
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
        let reshapes = self.reshapes_at(t);
        if !reshapes && !self.trims_at(t) && self.offset_amount.value_at(t) == 0.0 {
            return None;
        }
        let path = self.path_at(t);
        // A path about to be bent is walked finely, so its curves are still
        // curves once they have moved.
        let points = if reshapes {
            flatten_fine(&path)
        } else {
            flatten_path(&path)
        };
        if points.len() < 2 {
            return None;
        }
        Some(self.shaped(points, path.closed, t))
    }

    /// True when the smooth, the twist or the deform moves the path at `t`.
    fn reshapes_at(&self, t: f64) -> bool {
        self.twist.is_some()
            || self
                .smooth
                .as_ref()
                .is_some_and(|s| s.amount.value_at(t) > 0.0)
            || self
                .deform
                .as_ref()
                .is_some_and(|d| d.amount.value_at(t) != 0.0)
    }

    /// `points` once the smooth, the twist and the deform have had them, in
    /// that order. The control
    /// points of the item's own path are what both measure the shape by, so
    /// every contour of a combined run is moved the same way.
    fn reshaped(&self, points: Vec<(f64, f64)>, closed: bool, t: f64) -> Vec<(f64, f64)> {
        let mut points = match &self.smooth {
            Some(smooth) => smoothed(&points, closed, smooth.amount.value_at(t)),
            None => points,
        };
        // Straight edges have to be cut up before they can bend.
        points = densified(&points, FINE_STEP);
        let path = self.path_at(t);
        if let Some(twist) = &self.twist {
            twist.apply(&mut points, &path, t);
        }
        if let Some(deform) = &self.deform {
            points = deform.applied(&points, closed, &path, t);
        }
        points
    }

    /// One polyline pushed out by this item's offset and cut by its trim — the
    /// tail of [`trimmed_at`], shared so a **combined** run's contours
    /// are offset and trimmed by exactly the same arithmetic the item's own
    /// path is.
    fn shaped(&self, mut points: Vec<(f64, f64)>, closed: bool, t: f64) -> Vec<(f64, f64)> {
        if self.reshapes_at(t) {
            points = self.reshaped(points, closed, t);
        }
        let amount = self.offset_amount.value_at(t);
        if amount != 0.0 {
            points = offset_polyline(&points, amount, closed);
            if points.len() < 2 {
                return points;
            }
        }
        if !self.trims_at(t) {
            return points;
        }
        let (start, end) = (
            self.trim_start.value_at(t).clamp(0.0, 100.0),
            self.trim_end.value_at(t).clamp(0.0, 100.0),
        );
        // Degrees to per cent of the path: 360 is once round.
        let shift = self.trim_offset.value_at(t) / 3.6;
        if closed {
            // A closed path has a seam to run through, so the offset moves the
            // seam rather than the window: re-start the polyline `shift` per
            // cent along, and the ordinary trim then cuts one contiguous piece
            // even when it straddles where the first vertex used to be.
            crate::paint::trimmed(&rotated(&points, shift), start, end)
        } else {
            // An open path has two ends. Sliding the window off either of them
            // is the window running out of path, which is what clamping says.
            crate::paint::trimmed(&points, start + shift, end + shift)
        }
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
    /// `t` is here because the **repeater** puts art where the path is not, and
    /// where it puts it can be keyed. A trim needs no clock — it only ever
    /// takes art away, and the box stays the untrimmed one for the reason a
    /// paint stroke's bounds give.
    pub fn bounds(&self, t: f64) -> Option<(f64, f64, f64, f64)> {
        let path = self.path_at(t);
        let (x0, y0, x1, y1) = box_of(control_points(&path))?;
        // A twist keeps the art inside the box it was turned in, which can
        // reach past the path's own where the axis runs at a slant.
        let own = (x0, y0, x1, y1);
        let (x0, y0, x1, y1) = match &self.twist {
            Some(twist) => twist.bounds(&path, t).unwrap_or(own),
            None => own,
        };
        // A deform moves the outline no further than its amount says.
        let (x0, y0, x1, y1) = match &self.deform {
            Some(deform) => deform.bounds((x0, y0, x1, y1), own, t),
            None => (x0, y0, x1, y1),
        };
        // Half the stroke sits outside the path, so the box has to hold it.
        // Inflating **before** the copies are placed is what makes a scaled
        // copy's outline fit too: the copy's transform grows the margin with
        // the art.
        let half = if self.stroke.is_some() {
            self.stroke_width.value_at(t).max(0.0) / 2.0
        } else {
            0.0
        };
        // An outline pushed out of the path is art outside the path, so the
        // box holds it too. Pulled *in* it never needs more room, so
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
/// is not fixed by its source. It can also move as the art is *played*,
/// because a keyed repeater puts copies somewhere new each frame.
///
/// **A combined run is still measured member by member**. Every
/// boolean of two shapes lies inside the union of the two, so the box already
/// holds it; working the combine out again here would cost the boolean a
/// second time each frame to shave transparent pixels off the edge of a layer.
/// A subtract or an intersect therefore leaves the layer bigger than its
/// picture, which is the same generosity bounding a curve by its control
/// points already allows.
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

/// `contents` as they are at `t`: every keyed or expression-driven number
/// replaced by its value then, and every keyed path by its shape then.
///
/// The frame key and the renderer both read the art through this. The key
/// hashes the art, and stored keys say the same thing at every frame, so
/// without it every frame of a moving shape had one name. And the rasteriser
/// reads numbers without the layer's context, so an expression had no `time`.
/// Art with nothing moving comes back as it was, so a still shape keeps its
/// name and its pixels.
///
/// Walked as the file writes it, so no new field can be missed.
#[must_use]
pub fn contents_at<'a>(
    contents: &'a [ShapeItem],
    t: f64,
    context: &std::sync::Arc<crate::expression::ExpressionContext>,
) -> std::borrow::Cow<'a, [ShapeItem]> {
    use serde_json::Value;
    fn freeze(
        v: &mut Value,
        t: f64,
        context: &std::sync::Arc<crate::expression::ExpressionContext>,
    ) -> bool {
        let moves = v
            .get("animation")
            .and_then(Value::as_object)
            .is_some_and(|a| !a.contains_key("Static"));
        if moves {
            if let Ok(p) = serde_json::from_value::<Property>(v.clone()) {
                *v = Value::from(p.value_at_with_context(t, context.clone()));
                return true;
            }
        }
        match v {
            Value::Object(map) => map
                .values_mut()
                .fold(false, |moved, v| freeze(v, t, context) | moved),
            Value::Array(list) => list
                .iter_mut()
                .fold(false, |moved, v| freeze(v, t, context) | moved),
            _ => false,
        }
    }
    // Keyed paths are settled first and their keys dropped, so the walk below
    // never carries them. They hold nothing it changes, and a drawn morph can
    // be megabytes of keys, which every frame then sent through JSON and back.
    let paths_move = contents.iter().any(ShapeItem::path_is_animated);
    let settled: std::borrow::Cow<'a, [ShapeItem]> = if paths_move {
        let mut items = contents.to_vec();
        for item in items.iter_mut().filter(|item| item.path_is_animated()) {
            item.path = item.path_at(t).into_owned();
            item.path_keys.clear();
        }
        items.into()
    } else {
        contents.into()
    };
    let mut value = match serde_json::to_value(&*settled) {
        Ok(value) => value,
        Err(_) => return contents.into(),
    };
    if !freeze(&mut value, t, context) {
        return settled;
    }
    let Ok(frozen) = serde_json::from_value::<Vec<ShapeItem>>(value) else {
        return contents.into();
    };
    frozen.into()
}

/// The outlines a layer's art draws at `t`, as one run for an effect to walk
/// in the pixels of the layer's own picture: every line the picture strokes,
/// in list order, once a combine, an offset, a trim and the dashes have had
/// it, with each copy its repeater draws. An item with no paint is followed
/// too, which is how a guide path for points is drawn.
///
/// A line's vertices are its two ends and the item's own vertices that fall on
/// it. An offset or a combine leaves no telling which those were, so there a
/// vertex is wherever the line turns, and a curve has one at every joint.
#[must_use]
pub fn outline_at(contents: &[ShapeItem], t: f64) -> crate::mask::MaskPolyline {
    let mut out = crate::mask::MaskPolyline::default();
    // The picture's corner is the corner of the art's box.
    let Some((x0, y0, ..)) = contents_bounds(contents, t) else {
        return out;
    };
    let to_box = Affine::translation(-x0, -y0);
    for run in runs(contents) {
        let Some(art) = Art::at(contents, run, t) else {
            continue;
        };
        for (copy, _) in art.item.copies_at(t) {
            let place = copy.then(&to_box);
            let own = art.own(&place, t);
            for (line, _) in art.stroked(&art.placed(&place), copy.scale(), t) {
                if out.points.len() >= crate::fx::points::CAP_HARD as usize {
                    return out;
                }
                out.append(&walked(line, own.as_deref()));
            }
        }
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
/// `t` is the **layer's** own clock, the one its masks and its paint are
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

    for run in runs(contents) {
        // A combined run is drawn once, with the paint and the modifiers of the
        // **first** item in it; the ones after it lend their path and
        // nothing else. A run of one is every shape nobody has combined.
        let Some(item) = contents.get(run.start) else {
            continue;
        };
        let opacity = (item.opacity.value_at(t).clamp(0.0, 100.0) / 100.0) as f32;
        if opacity <= 0.0 {
            continue;
        }
        let Some(art) = Art::at(contents, run, t) else {
            continue;
        };

        // The copies the repeater asks for, drawn **last first** so the
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
            let placed = art.placed(&to_box);

            if let Some(fill) = item.fill.as_ref().map(|c| colour::at(c, t)) {
                // All the contours at once, so a boolean's hole is a hole: the
                // crossings have to be counted across the whole shape, not one
                // ring at a time.
                let coverage = crate::mask::rasterise_paths(&placed, w, h, sx, sy, true);
                // A gradient fill is the same coverage painted with a
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

            let width = item.stroke_width.value_at(t);
            if let (Some(stroke), true) =
                (item.stroke.as_ref().map(|c| colour::at(c, t)), width > 0.0)
            {
                // A stroke is a brush run along the path, which is exactly what the
                // paint rasteriser already does — one widened-path implementation
                // for both, rather than two that can disagree.
                // The outline follows the trimmed piece, and it is drawn **open**
                // whatever the path was: a trim is what turns a closed ring into a
                // stroke with two ends. A **boolean's** contour is a ring and is
                // outlined all the way round, so it comes back to its first point.
                // A copy is a scaled **drawing**, not a scaled path: its outline
                // and its dashes grow with it, or a copy at half size would be a
                // shape with an outline twice as heavy.
                let pieces = art.stroked(&placed, copy.scale(), t);
                let brushes: Vec<crate::paint::PaintStroke> = pieces
                    .into_iter()
                    .filter(|(p, _)| p.len() >= 2)
                    .map(|(points, pressures)| crate::paint::PaintStroke {
                        id: item.id,
                        name: item.name.clone(),
                        points,
                        // A vector outline is not a gesture: there is no stylus
                        // behind it, and its width is the item's own, narrowed
                        // along the line by a taper where it has one.
                        pressures,
                        colour: stroke,
                        width: width * copy.scale(),
                        // A vector outline has a hard edge; the rasteriser keeps
                        // half a pixel of falloff whatever this says, which is the
                        // anti-aliasing rather than a soft brush.
                        hardness: 1.0,
                        // A vector outline is round-capped and round-joined, which
                        // is what the round brush already draws.
                        shape: crate::paint::BrushShape::Round,
                        // The item's own opacity, faded by this copy's share of the
                        // repeater's ramp — the same number the fill above
                        // multiplied its coverage by, in the per cent the brush
                        // reads it in.
                        opacity: item.opacity.value_at(t) * ramp,
                        // A vector outline is drawn whole; a shape item's own trim
                        // paths are a shape-layer feature, not the brush's,
                        // so the whole path every time and no clock to read.
                        start: crate::anim::Property::zero(),
                        end: crate::anim::Property::fixed(100.0),
                        mode: crate::paint::PaintMode::Paint,
                        // A shape item's outline lays its colour down; a blend of
                        // its own would be a shape-layer feature.
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

impl Twist {
    /// The axis's direction, the direction across it, and the centre.
    fn frame(&self, t: f64) -> ((f64, f64), (f64, f64), (f64, f64)) {
        let (sin, cos) = self.axis.value_at(t).to_radians().sin_cos();
        (
            (cos, sin),
            (-sin, cos),
            (self.centre_x.value_at(t), self.centre_y.value_at(t)),
        )
    }

    /// How far `path` reaches back and forward along the axis from the
    /// centre, and how far across it at most.
    fn reach(&self, path: &BezierPath, t: f64) -> (f64, f64, f64) {
        let (a, n, c) = self.frame(t);
        control_points(path).fold((0.0f64, 0.0f64, 0.0f64), |r, p| {
            let d = (p.0 - c.0, p.1 - c.1);
            let along = d.0 * a.0 + d.1 * a.1;
            let across = d.0 * n.0 + d.1 * n.1;
            (r.0.min(along), r.1.max(along), r.2.max(across.abs()))
        })
    }

    /// Twist `points` in place. Each point keeps its place along the axis;
    /// its distance across it is what you see of that distance once its
    /// slice of the shape has turned, which is none at the back end of
    /// `path` and the whole angle at the front, plus the rotation.
    fn apply(&self, points: &mut [(f64, f64)], path: &BezierPath, t: f64) {
        let (a, n, c) = self.frame(t);
        let (back, forward, _) = self.reach(path, t);
        let span = forward - back;
        let per = if span > 1e-9 {
            self.angle.value_at(t).to_radians() / span
        } else {
            0.0
        };
        let rotation = self.rotation.value_at(t).to_radians();
        for p in points {
            let d = (p.0 - c.0, p.1 - c.1);
            let along = d.0 * a.0 + d.1 * a.1;
            let across = (d.0 * n.0 + d.1 * n.1) * ((along - back) * per + rotation).cos();
            *p = (
                c.0 + along * a.0 + across * n.0,
                c.1 + along * a.1 + across * n.1,
            );
        }
    }

    /// The box the twisted `path` stays inside: its reach along the axis, and
    /// as far across it either way as it ever reached.
    fn bounds(&self, path: &BezierPath, t: f64) -> Option<(f64, f64, f64, f64)> {
        let (a, n, c) = self.frame(t);
        let (back, forward, across) = self.reach(path, t);
        let corner = |along: f64, side: f64| {
            (
                c.0 + along * a.0 + side * n.0,
                c.1 + along * a.1 + side * n.1,
            )
        };
        box_of(
            [
                corner(back, -across),
                corner(back, across),
                corner(forward, -across),
                corner(forward, across),
            ]
            .into_iter(),
        )
    }
}

/// The most places one contour reads a deform's field at.
const MAX_FIELD_SAMPLES: f64 = 4096.0;

impl Deform {
    /// The field's value at each of `count` samples, read where they fall
    /// on the path, in −1..=1.
    fn field(&self, ring: &[(f64, f64)], arc: &[f64], gap: f64, count: usize, t: f64) -> Vec<f64> {
        let scale = self.scale.value_at(t).max(1.0);
        let depth = (self.evolution.value_at(t) / 360.0) as f32;
        let seed = self.seed.value_at(t).round() as i64 as u32;
        (0..count)
            .map(|k| {
                let p = point_at(ring, arc, k as f64 * gap);
                let v = crate::fx::noise::perlin3(
                    seed,
                    0,
                    (p.0 / scale) as f32,
                    (p.1 / scale) as f32,
                    depth,
                    0,
                );
                // Held to one, which is what keeps the layer's box honest.
                f64::from(v).clamp(-1.0, 1.0)
            })
            .collect()
    }

    /// `points` moved by the field. `path` gives the middle a Scale deform
    /// measures from.
    fn applied(
        &self,
        points: &[(f64, f64)],
        closed: bool,
        path: &BezierPath,
        t: f64,
    ) -> Vec<(f64, f64)> {
        let amount = self.amount.value_at(t);
        let (ring, repeat) = ring_of(points, closed);
        let n = ring.len();
        let (arc, total) = arc_of(ring, closed);
        if n < 2 || amount == 0.0 || total <= 0.0 {
            return points.to_vec();
        }
        let scale = self.scale.value_at(t).max(1.0);
        let detail = self.detail.value_at(t).clamp(0.1, 100.0);
        let pieces = (total * detail / scale)
            .ceil()
            .clamp(if closed { 3.0 } else { 1.0 }, MAX_FIELD_SAMPLES) as usize;
        let gap = total / pieces as f64;
        // A closed ring's last sample leads back into its first; an open path
        // has one at each end.
        let count = if closed { pieces } else { pieces + 1 };
        let field = self.field(ring, &arc, gap, count, t);
        let sample = |k: isize| {
            let k = if closed {
                k.rem_euclid(count as isize)
            } else {
                k.clamp(0, count as isize - 1)
            };
            field.get(k as usize).copied().unwrap_or(0.0)
        };
        // Eased between the samples, so the outline has no corner where one
        // sample hands over to the next.
        let read = |s: f64| {
            let u = s / gap;
            let k = u.floor() as isize;
            let f = u - u.floor();
            let (p0, p1, p2, p3) = (sample(k - 1), sample(k), sample(k + 1), sample(k + 2));
            let v = 0.5
                * (2.0 * p1
                    + (p2 - p0) * f
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * f * f
                    + (3.0 * (p1 - p2) + p3 - p0) * f * f * f);
            v.clamp(-1.0, 1.0)
        };
        let middle = box_of(control_points(path))
            .map(|b| ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0))
            .unwrap_or_default();
        // Out is decided by the ring's own winding, as the offset decides it.
        let out = if closed && shoelace(ring) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let mut moved: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let p = ring[i];
                let v = read(arc[i]);
                if self.mode == 1 {
                    let f = 1.0 + amount / 100.0 * v;
                    return (
                        middle.0 + (p.0 - middle.0) * f,
                        middle.1 + (p.1 - middle.1) * f,
                    );
                }
                let before = match i {
                    0 if closed => ring[n - 1],
                    0 => p,
                    _ => ring[i - 1],
                };
                let after = match ring.get(i + 1) {
                    Some(&q) => q,
                    None if closed => ring[0],
                    None => p,
                };
                let (dx, dy) = (after.0 - before.0, after.1 - before.1);
                let len = dx.hypot(dy);
                if len <= 1e-12 {
                    return p;
                }
                let push = out * amount * v / len;
                (p.0 + dy * push, p.1 - dx * push)
            })
            .collect();
        if repeat {
            moved.push(moved[0]);
        }
        moved
    }

    /// `twisted`, the box the art is in before the deform, grown to hold
    /// what the deform can do to it. `own` is the path's own box, whose middle
    /// a Scale deform measures from.
    fn bounds(
        &self,
        twisted: (f64, f64, f64, f64),
        own: (f64, f64, f64, f64),
        t: f64,
    ) -> (f64, f64, f64, f64) {
        let amount = self.amount.value_at(t).abs();
        let (x0, y0, x1, y1) = twisted;
        if self.mode != 1 {
            return (x0 - amount, y0 - amount, x1 + amount, y1 + amount);
        }
        let (mx, my) = ((own.0 + own.2) / 2.0, (own.1 + own.3) / 2.0);
        let grow = 1.0 + amount / 100.0;
        let rx = (x0 - mx).abs().max((x1 - mx).abs()) * grow;
        let ry = (y0 - my).abs().max((y1 - my).abs()) * grow;
        (mx - rx, my - ry, mx + rx, my + ry)
    }
}

/// `points` with every point averaged with the path either side of it,
/// `amount` pixels each way: the path walked into evenly spaced points, then
/// three running averages, which together weigh the neighbours like a bell
/// curve. An open path's ends stay put, the window closing in towards them.
fn smoothed(points: &[(f64, f64)], closed: bool, amount: f64) -> Vec<(f64, f64)> {
    let (ring, repeat) = ring_of(points, closed);
    let (arc, total) = arc_of(ring, closed);
    if amount <= 0.0 || ring.len() < 3 || total <= 0.0 {
        return points.to_vec();
    }
    let step = (amount / 8.0)
        .clamp(0.25, FINE_STEP)
        .max(total / MAX_FINE_POINTS);
    let pieces = (total / step).ceil().max(3.0) as usize;
    let n = if closed { pieces } else { pieces + 1 };
    let gap = total / pieces as f64;
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    for k in 0..n {
        let p = point_at(ring, &arc, k as f64 * gap);
        xs.push(p.0);
        ys.push(p.1);
    }
    let reach = ((amount / (3.0 * gap)).round() as usize).max(1);
    for _ in 0..3 {
        xs = averaged(&xs, reach, closed);
        ys = averaged(&ys, reach, closed);
    }
    let mut out: Vec<(f64, f64)> = xs.into_iter().zip(ys).collect();
    if repeat {
        out.push(out[0]);
    }
    out
}

/// Each of `values` replaced by the mean of itself and `reach` either side,
/// wrapping round when `closed`. Open, the window narrows towards each end so
/// the ends themselves never move.
fn averaged(values: &[f64], reach: usize, closed: bool) -> Vec<f64> {
    let n = values.len();
    // Running totals, so the cost does not grow with the reach.
    let mut sums = Vec::with_capacity(n + 1);
    sums.push(0.0);
    for v in values {
        sums.push(sums[sums.len() - 1] + v);
    }
    let all = sums[n];
    if closed {
        let reach = reach.min((n - 1) / 2);
        let width = (2 * reach + 1) as f64;
        (0..n)
            .map(|i| {
                let (from, to) = ((i + n - reach) % n, (i + reach) % n);
                let sum = if from <= to {
                    sums[to + 1] - sums[from]
                } else {
                    all - sums[from] + sums[to + 1]
                };
                sum / width
            })
            .collect()
    } else {
        (0..n)
            .map(|i| {
                let r = reach.min(i).min(n - 1 - i);
                (sums[i + r + 1] - sums[i - r]) / (2 * r + 1) as f64
            })
            .collect()
    }
}

/// How many straight steps a round join is drawn in, per quarter turn.
///
/// Four is smooth at the sizes an offset outline is drawn at, and a count taken
/// from the angle rather than the size keeps the answer identical on every
/// machine (docs/14 §determinism).
const JOIN_STEPS_PER_QUARTER: f64 = 4.0;

/// `points` pushed `amount` layer pixels **out** of itself, corners rounded —
/// After Effects' Offset Paths, with the one join this crate draws.
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

pub mod colour;
pub mod geometry;
mod raster;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
