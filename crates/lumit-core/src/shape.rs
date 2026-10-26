//! Shape layers: vector art as a layer's own picture, and the rasteriser that
//! draws one.
//!
//! # In plain terms
//!
//! A **mask** is a path drawn *on* another layer, deciding which of its pixels
//! show. A **shape layer** is a path that **is** the picture: a rectangle, an
//! ellipse, a drawn path, filled, outlined, and made of numbers rather than
//! pixels, so it stays crisp at any size. After Effects makes one whenever you
//! drag a shape tool with nothing selected, and this is the layer kind that lets
//! Lumit do the same.
//!
//! **The path type is the mask's.** One `BezierPath` in the document, one set of
//! maths, one vertex type crossing the bridge. A shape's path and a mask's path
//! differ in what they *do*, not in what they are, which is why the shape tools
//! could draw both from the same geometry from the day they landed.
//!
//! **The layer's own size is the art's bounding box**, and it changes as the art
//! is edited. Every other layer kind has a size fixed by its source; this is the
//! first that does not, and anything caching "how big is this layer" has to
//! follow the document's revision rather than assume.
//!
//! **The contents is a list of entries** ([`modifier::ShapeEntry`]): pieces of
//! art, the modifiers that act on the art above them, and groups of either,
//! the way After Effects lays a shape layer out. An item carries its path and
//! its paint; the trim, the offset, the repeater, the twist, the deform and
//! the smooth are entries of their own, and so stack and reorder. The
//! **combine** stays on the item: it says how this item's path joins the item
//! above it, and the run that makes is drawn once with the first item's paint.
//! Files from before the entries read as they did; `modifier` says how.
//!
//! **What is deliberately not here.** Wiggle paths, and line joins other than
//! round.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::anim::Property;
use crate::mask::BezierPath;
use crate::model::LinearColour;
use crate::pixels::over;

use geometry::{box_of, control_points, flatten_fine, flatten_path, Affine};
use modifier::{ShapeEntry, ShapeModifier};
use raster::{runs, walked, Art};

/// One piece of vector art: a path, and how it is painted.
///
/// What is done to the path, the trim and the repeater and the rest, is an
/// entry after it in the contents ([`modifier::ShapeModifier`]), not a field
/// here.
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
    /// **Stroke taper**: the outline thinning towards its ends. `None` is
    /// the full width all the way along.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taper: Option<Taper>,
    /// **A parametric shape**: the rectangle, ellipse, polygon or star this
    /// item's path is made from at every frame, so its size and corners key.
    /// `None` is a drawn path. While it is set, [`path`](Self::path) holds the
    /// shape at the head of the layer for a Lumit that reads no primitive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primitive: Option<primitive::ShapePrimitive>,
    /// Unknown fields from newer Lumit versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
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

/// True for the zeroth reading of a choice — a flat fill, an item combined with
/// nothing — and so nothing written to the file.
pub(super) fn is_flat(kind: &u32) -> bool {
    *kind == 0
}

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
            taper: None,
            primitive: None,
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
        match &self.primitive {
            Some(p) => std::borrow::Cow::Owned(p.path_at(t)),
            None => crate::mask::path_at(&self.path_keys, &self.path, t),
        }
    }

    /// Whether this item's shape moves: keyframed, or made from numbers that
    /// move.
    pub fn path_is_animated(&self) -> bool {
        match &self.primitive {
            Some(p) => p.is_animated(),
            None => !self.path_keys.is_empty(),
        }
    }

    /// The item's art at `t` as a polyline, once `mods` have had it: `None`
    /// when none of them changes the path (draw the bezier instead), and
    /// `Some(empty)` when a trim leaves nothing at all.
    ///
    /// The check matters: an unmodified shape is rasterised from its
    /// **bezier** rather than from a polyline of it. Flattening is exact
    /// enough to stroke with and not exact enough to make the identity case
    /// draw different pixels than it did before there were modifiers.
    pub(super) fn trimmed_at(&self, mods: &[&ShapeModifier], t: f64) -> Option<Vec<(f64, f64)>> {
        if !modifier::moves(mods, t) {
            return None;
        }
        let path = self.path_at(t);
        // A path about to be bent is walked finely, so its curves are still
        // curves once they have moved.
        let points = if modifier::reshapes(mods, t) {
            flatten_fine(&path)
        } else {
            flatten_path(&path)
        };
        if points.len() < 2 {
            return None;
        }
        Some(modifier::shaped(mods, points, path.closed, &path, t))
    }

    /// The item's bounding box in layer coordinates at `t`, with `mods`
    /// acting on it: its outline and its repeated copies included, or `None`
    /// when its path has no vertices.
    ///
    /// The **control points** bound the curve rather than the curve itself: a
    /// cubic never leaves its own control hull, so this is correct and never
    /// too small. It can be a little generous on a strongly curved path, which
    /// costs a few transparent pixels and no correctness.
    ///
    /// `t` is here because the **repeater** puts art where the path is not, and
    /// where it puts it can be keyed. A trim needs no clock, it only ever
    /// takes art away, and the box stays the untrimmed one for the reason a
    /// paint stroke's bounds give.
    pub fn bounds(&self, mods: &[&ShapeModifier], t: f64) -> Option<(f64, f64, f64, f64)> {
        let path = self.path_at(t);
        let own = box_of(control_points(&path))?;
        let ((x0, y0, x1, y1), margin) = modifier::grown(mods, own, &path, t);
        // Half the stroke sits outside the path, so the box has to hold it.
        // Inflating **before** the copies are placed is what makes a scaled
        // copy's outline fit too: the copy's transform grows the margin with
        // the art.
        let half = if self.stroke.is_some() {
            self.stroke_width.value_at(t).max(0.0) / 2.0
        } else {
            0.0
        };
        let half = half + margin;
        let (x0, y0, x1, y1) = (x0 - half, y0 - half, x1 + half, y1 + half);

        // Every copy's box, unioned. One copy and the identity, every shape
        // nobody has repeated, hands back exactly the box above.
        let mut whole: Option<(f64, f64, f64, f64)> = None;
        for (m, _) in modifier::copies_at(mods, t) {
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
/// there is no art in it: the layer's **natural size**.
///
/// This is the number the wireframe, hit-testing and the transform all read, and
/// it moves when the art is edited: a shape layer is the first kind whose size
/// is not fixed by its source. It can also move as the art is *played*,
/// because a keyed repeater puts copies somewhere new each frame.
///
/// **A combined run is still measured member by member**, each with the
/// modifiers the run is drawn through. Every boolean of two shapes lies
/// inside the union of the two, so the box already holds it; working the
/// combine out again here would cost the boolean a second time each frame to
/// shave transparent pixels off the edge of a layer. A subtract or an
/// intersect therefore leaves the layer bigger than its picture, which is the
/// same generosity bounding a curve by its control points already allows.
pub fn contents_bounds(contents: &[ShapeEntry], t: f64) -> Option<(f64, f64, f64, f64)> {
    let mut out: Option<(f64, f64, f64, f64)> = None;
    for art in modifier::lowered(contents) {
        let Some(b) = art.item.bounds(&art.mods, t) else {
            continue;
        };
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
    contents: &'a [ShapeEntry],
    t: f64,
    context: &std::sync::Arc<crate::expression::ExpressionContext>,
) -> std::borrow::Cow<'a, [ShapeEntry]> {
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
    let paths_move = modifier::items(contents)
        .iter()
        .any(|item| item.path_is_animated());
    let settled: std::borrow::Cow<'a, [ShapeEntry]> = if paths_move {
        let mut entries = contents.to_vec();
        modifier::items_mut(&mut entries, &mut |item| {
            if item.path_is_animated() {
                item.path = item.path_at(t).into_owned();
                item.path_keys.clear();
            }
        });
        entries.into()
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
    let Ok(frozen) = serde_json::from_value::<Vec<ShapeEntry>>(value) else {
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
pub fn outline_at(contents: &[ShapeEntry], t: f64) -> crate::mask::MaskPolyline {
    let mut out = crate::mask::MaskPolyline::default();
    // The picture's corner is the corner of the art's box.
    let Some((x0, y0, ..)) = contents_bounds(contents, t) else {
        return out;
    };
    let to_box = Affine::translation(-x0, -y0);
    let items = modifier::lowered(contents);
    for run in runs(&items) {
        let Some(art) = Art::at(&items, run, t) else {
            continue;
        };
        for (copy, _) in modifier::copies_at(art.mods, t) {
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
    contents: &[ShapeEntry],
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

    let items = modifier::lowered(contents);
    for run in runs(&items) {
        // A combined run is drawn once, with the paint and the modifiers of the
        // **first** item in it; the ones after it lend their path and
        // nothing else. A run of one is every shape nobody has combined.
        let Some(item) = items.get(run.start).map(|art| art.item) else {
            continue;
        };
        let opacity = (item.opacity.value_at(t).clamp(0.0, 100.0) / 100.0) as f32;
        if opacity <= 0.0 {
            continue;
        }
        let Some(art) = Art::at(&items, run, t) else {
            continue;
        };

        // The copies the repeater asks for, drawn **last first** so the
        // original ends up on top of the copies made from it — which is what
        // After Effects draws, and the only order in which turning the count up
        // does not hide the shape you already had.
        for (copy, ramp) in modifier::copies_at(art.mods, t).into_iter().rev() {
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

mod bend;
pub mod colour;
pub mod expand;
pub mod geometry;
pub mod modifier;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod modifier_tests;
pub mod primitive;
mod raster;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
