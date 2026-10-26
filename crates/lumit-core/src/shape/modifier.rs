//! Modifiers as entries in a shape layer's contents.
//!
//! A layer's contents is an ordered list of three kinds of entry: a piece of
//! art ([`ShapeItem`]), a modifier, and a group of either. **A modifier acts
//! on every piece of art above it in its group**, the way After Effects'
//! Repeater and Trim Paths do, and the art inside a group is also acted on by
//! the modifiers after that group in the list outside it. Two modifiers of
//! one kind stack: a repeater below a repeater repeats the copies, which is
//! how a row becomes a grid.
//!
//! Nothing here draws. [`lowered`] hands each item the modifiers that act on
//! it, in the order they apply, and the rasteriser draws the item through
//! them with [`shaped`] and [`copies_at`].
//!
//! **Older files kept the modifiers as fields on each item**, applied in one
//! fixed order. [`read_entries`] lifts those fields into entries as the file
//! is read, in that same order, placed after the item (or after the run of
//! items it was drawn as one with). Where an entry there would also reach
//! art above the item, the item and its modifiers are wrapped in a group, so
//! the picture is the one the file was saved with.

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use super::bend::{offset_polyline, repeat_step, smoothed};
use super::geometry::{densified, rotated, Affine, FINE_STEP};
use super::{is_flat, ShapeItem};
use crate::anim::{Animation, Property};
use crate::mask::BezierPath;

/// One entry in a shape layer's contents.
///
/// Untagged in the file: a modifier is told by its `modifier` key, a group by
/// its `entries`, and anything else is the art an older file already held.
// Nearly every entry is an item, so boxing the item to slim the enum would
// cost an allocation per piece of art each frame copies.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ShapeEntry {
    Modifier(ShapeModifier),
    Group(ShapeGroup),
    Item(ShapeItem),
}

/// A group: entries drawn in order, whose modifiers act only inside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeGroup {
    pub id: Uuid,
    pub name: String,
    #[serde(deserialize_with = "read_entries")]
    pub entries: Vec<ShapeEntry>,
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A modifier entry: what it is, and what it is called in the Timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeModifier {
    pub id: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub kind: ModifierKind,
}

/// The modifiers there are, each with its numbers. Written as
/// `"modifier": "repeat"` beside the numbers.
// A repeater is ten properties and a smooth is one; a layer holds a few of
// either, so the odd size is not worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "modifier", rename_all = "snake_case")]
pub enum ModifierKind {
    Repeat(Repeater),
    Trim(Trim),
    Offset(Offset),
    Twist(Twist),
    Deform(Deform),
    Smooth(Smooth),
}

impl ModifierKind {
    /// What a modifier of this kind is called when nobody has named it.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Repeat(_) => "Repeater",
            Self::Trim(_) => "Trim paths",
            Self::Offset(_) => "Offset paths",
            Self::Twist(_) => "Twist",
            Self::Deform(_) => "Deform",
            Self::Smooth(_) => "Smooth",
        }
    }

    /// True when this modifier bends the path at `t`: a smooth with an
    /// amount, any twist, a deform with an amount. A bent path is walked
    /// finely first, so its curves are still curves once they have moved.
    fn bends(&self, t: f64) -> bool {
        match self {
            Self::Smooth(s) => s.amount.value_at(t) > 0.0,
            Self::Twist(_) => true,
            Self::Deform(d) => d.amount.value_at(t) != 0.0,
            _ => false,
        }
    }

    /// True when this modifier pushes the outline off the path at `t`.
    fn pushes(&self, t: f64) -> bool {
        matches!(self, Self::Offset(o) if o.amount.value_at(t) != 0.0)
    }

    /// True when this modifier cuts the path at `t`.
    fn trims(&self, t: f64) -> bool {
        matches!(self, Self::Trim(trim) if trim.trims_at(t))
    }
}

/// **The repeater**: how many copies of the art are drawn, and the
/// transform each copy is one more step of.
///
/// `copies` is a count, rounded and held to 1..[`MAX_COPIES`]. `offset` says
/// which copy the original is: copy *k* runs from the offset, so a negative
/// offset puts copies *behind* the original. The step moves by `position_x`
/// and `position_y` layer pixels, turns by `rotation` degrees and scales by
/// `scale` per cent, all about the anchor. The copies fade evenly from
/// `start_opacity` to `end_opacity`, per cent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Repeater {
    #[serde(default = "three", with = "crate::mask::still_or_keyed")]
    pub copies: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub offset: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub anchor_x: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub anchor_y: Property,
    #[serde(default = "hundred", with = "crate::mask::still_or_keyed")]
    pub position_x: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub position_y: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub rotation: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub scale: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub start_opacity: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub end_opacity: Property,
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// **Trim paths**: where along the path the art begins and ends, as a per
/// cent of the path's own **arc length**, and how far the pair is slid along
/// it in degrees (360 is once round).
///
/// Per cent of length rather than of vertex count, for the reason a paint
/// stroke's write-on gives: the eye watches length. The trim cuts the fill as
/// well as the outline. `end` at or below `start` draws nothing at all, which
/// is what the first frame of a write-on looks like rather than an error. A
/// closed path **wraps** under the offset and an open one does not, because
/// it has no seam to run through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub start: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    pub end: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub offset: Property,
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// **Offset paths**: how far the outline is pushed **out** of the path, in
/// layer pixels. Negative pulls it in. The corners are round, which is the
/// one join this crate draws, and self-intersections are left in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offset {
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub amount: Property,
    /// Unknown fields from newer Lumit versions, preserved on load/save.
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
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A smooth: every point of the path averaged with its neighbours either
/// side, `amount` pixels of path each way, so jags and corners round off
/// along the whole of it. An average never leaves the points it was taken
/// over, so the path never bulges past itself; it shrinks a little instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Smooth {
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub amount: Property,
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn three() -> Property {
    Property::fixed(3.0)
}

fn four() -> Property {
    Property::fixed(4.0)
}

fn hundred() -> Property {
    Property::fixed(100.0)
}

/// The most copies one repeater draws.
///
/// Every copy is a rasteriser pass of its own over the whole layer, so the
/// count is the frame's cost written as a number. A hundred is past what a row
/// of things or a ring of ticks asks for and still inside a frame; a count past
/// it is **held** here rather than refused, because the number is a slider and a
/// slider that stops is kinder than a frame that does not arrive. Lifting the
/// ceiling means a rasteriser that can be pointed at one copy's own box rather
/// than the whole layer, which is a change to the mask rasteriser, not here.
pub const MAX_COPIES: i64 = 100;

/// The most copies stacked repeaters draw of one piece of art between them.
/// A grid of thirty by thirty is inside it; past it the later repeater's last
/// copies are left out rather than the frame.
pub const MAX_COPIES_DRAWN: usize = 1000;

impl Repeater {
    /// The copies this repeater draws at `t`, each with its transform and the
    /// share of opacity to draw it at (0..1). One copy, identity, full
    /// opacity, for a count of one.
    fn copies_at(&self, t: f64) -> Vec<(Affine, f64)> {
        let copies = self
            .copies
            .value_at(t)
            .round()
            .clamp(1.0, MAX_COPIES as f64) as i64;
        if copies <= 1 {
            return vec![(Affine::IDENTITY, 1.0)];
        }
        let offset = self
            .offset
            .value_at(t)
            .round()
            .clamp(-(MAX_COPIES as f64), MAX_COPIES as f64) as i64;
        let step = repeat_step(
            (self.position_x.value_at(t), self.position_y.value_at(t)),
            self.rotation.value_at(t),
            self.scale.value_at(t) / 100.0,
            (self.anchor_x.value_at(t), self.anchor_y.value_at(t)),
        );
        let (from, to) = (
            self.start_opacity.value_at(t).clamp(0.0, 100.0) / 100.0,
            self.end_opacity.value_at(t).clamp(0.0, 100.0) / 100.0,
        );

        // The first copy is step^offset, and every one after it is one more
        // step, so the offset decides which copy the original geometry is, and
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
}

impl Trim {
    /// True when the trim at `t` would change the art. An untrimmed shape is
    /// rasterised from its **bezier** rather than from a polyline of it, so
    /// the identity case draws the pixels it always drew.
    fn trims_at(&self, t: f64) -> bool {
        self.start.value_at(t) > 0.0
            || self.end.value_at(t) < 100.0
            || self.offset.value_at(t) != 0.0
    }

    /// `points` cut to the piece between the two ends, slid by the offset.
    fn cut(&self, points: &[(f64, f64)], closed: bool, t: f64) -> Vec<(f64, f64)> {
        let (start, end) = (
            self.start.value_at(t).clamp(0.0, 100.0),
            self.end.value_at(t).clamp(0.0, 100.0),
        );
        // Degrees to per cent of the path: 360 is once round.
        let shift = self.offset.value_at(t) / 3.6;
        if closed {
            // A closed path has a seam to run through, so the offset moves the
            // seam rather than the window: re-start the polyline `shift` per
            // cent along, and the ordinary trim then cuts one contiguous piece
            // even when it straddles where the first vertex used to be.
            crate::paint::trimmed(&rotated(points, shift), start, end)
        } else {
            // An open path has two ends. Sliding the window off either of them
            // is the window running out of path, which is what clamping says.
            crate::paint::trimmed(points, start + shift, end + shift)
        }
    }
}

/// One piece of art as the rasteriser sees it: the item, the modifiers
/// acting on it in the order they apply, and whether it joins the item above
/// it into one drawing (its [`combine`](ShapeItem::combine), when the entry
/// above it is an item).
pub struct Lowered<'a> {
    pub item: &'a ShapeItem,
    pub mods: Vec<&'a ShapeModifier>,
    pub joins: bool,
}

/// Every piece of art in `entries`, in the order it is drawn, each with the
/// modifiers that act on it.
#[must_use]
pub fn lowered(entries: &[ShapeEntry]) -> Vec<Lowered<'_>> {
    let mut out = Vec::new();
    lower(entries, &[], &mut out);
    out
}

fn lower<'a>(entries: &'a [ShapeEntry], outer: &[&'a ShapeModifier], out: &mut Vec<Lowered<'a>>) {
    for (i, entry) in entries.iter().enumerate() {
        // The modifiers after this entry in its group, then the ones after
        // the group outside.
        let below = || {
            entries[i + 1..]
                .iter()
                .filter_map(|e| match e {
                    ShapeEntry::Modifier(m) => Some(m),
                    _ => None,
                })
                .chain(outer.iter().copied())
                .collect::<Vec<_>>()
        };
        match entry {
            ShapeEntry::Item(item) => out.push(Lowered {
                item,
                mods: below(),
                joins: item.combine != 0 && i > 0 && matches!(entries[i - 1], ShapeEntry::Item(_)),
            }),
            ShapeEntry::Group(group) => lower(&group.entries, &below(), out),
            ShapeEntry::Modifier(_) => {}
        }
    }
}

/// Every piece of art in `entries`, groups opened, in list order.
#[must_use]
pub fn items(entries: &[ShapeEntry]) -> Vec<&ShapeItem> {
    lowered(entries).into_iter().map(|l| l.item).collect()
}

/// The piece of art `id` names, wherever it sits.
pub fn item_mut(entries: &mut [ShapeEntry], id: Uuid) -> Option<&mut ShapeItem> {
    for entry in entries {
        match entry {
            ShapeEntry::Item(item) if item.id == id => return Some(item),
            ShapeEntry::Group(group) => {
                if let Some(found) = item_mut(&mut group.entries, id) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// `change` run over every piece of art in `entries`, groups opened.
pub fn items_mut(entries: &mut [ShapeEntry], change: &mut impl FnMut(&mut ShapeItem)) {
    for entry in entries {
        match entry {
            ShapeEntry::Item(item) => change(item),
            ShapeEntry::Group(group) => items_mut(&mut group.entries, change),
            ShapeEntry::Modifier(_) => {}
        }
    }
}

/// True when any of `mods` bends the path at `t`, so it has to be walked
/// finely before it moves.
pub(super) fn reshapes(mods: &[&ShapeModifier], t: f64) -> bool {
    mods.iter().any(|m| m.kind.bends(t))
}

/// True when any of `mods` moves the outline off the item's own vertices at
/// `t`: bent or pushed, though not merely cut.
pub(super) fn displaces(mods: &[&ShapeModifier], t: f64) -> bool {
    mods.iter().any(|m| m.kind.bends(t) || m.kind.pushes(t))
}

/// True when any of `mods` changes the path at `t`. False is the identity
/// case, which draws the bezier itself.
pub(super) fn moves(mods: &[&ShapeModifier], t: f64) -> bool {
    mods.iter()
        .any(|m| m.kind.bends(t) || m.kind.pushes(t) || m.kind.trims(t))
}

/// `points` once `mods` have had them, in list order. `path` is the item's
/// own shape at `t`, which a twist and a deform measure themselves against.
///
/// A path about to bend is cut into fine steps first, once, so a straight
/// edge has points along it to move; a smooth resamples the path itself and
/// is cut after. An offset that leaves fewer than two points ends the run.
pub(super) fn shaped(
    mods: &[&ShapeModifier],
    mut points: Vec<(f64, f64)>,
    closed: bool,
    path: &BezierPath,
    t: f64,
) -> Vec<(f64, f64)> {
    let mut dense = false;
    let mut densify = |points: &mut Vec<(f64, f64)>| {
        if !dense {
            *points = densified(points, FINE_STEP);
            dense = true;
        }
    };
    for m in mods {
        match &m.kind {
            ModifierKind::Smooth(s) => {
                let amount = s.amount.value_at(t);
                if amount > 0.0 {
                    points = smoothed(&points, closed, amount);
                    densify(&mut points);
                }
            }
            ModifierKind::Twist(w) => {
                densify(&mut points);
                w.apply(&mut points, path, t);
            }
            ModifierKind::Deform(d) => {
                if d.amount.value_at(t) != 0.0 {
                    densify(&mut points);
                    points = d.applied(&points, closed, path, t);
                }
            }
            ModifierKind::Offset(o) => {
                let amount = o.amount.value_at(t);
                if amount != 0.0 {
                    points = offset_polyline(&points, amount, closed);
                    if points.len() < 2 {
                        return points;
                    }
                }
            }
            ModifierKind::Trim(trim) => {
                if trim.trims_at(t) {
                    points = trim.cut(&points, closed, t);
                }
            }
            ModifierKind::Repeat(_) => {}
        }
    }
    points
}

/// The copies `mods` draw of one piece of art at `t`, each with its
/// transform and the share of opacity to draw it at. Stacked repeaters
/// multiply: every copy of the later one is made of every copy of the
/// earlier, which is the grid. One copy, identity, full opacity, for art
/// nobody has repeated.
#[must_use]
pub fn copies_at(mods: &[&ShapeModifier], t: f64) -> Vec<(Affine, f64)> {
    let mut out = vec![(Affine::IDENTITY, 1.0)];
    for m in mods {
        let ModifierKind::Repeat(repeater) = &m.kind else {
            continue;
        };
        let copies = repeater.copies_at(t);
        if copies.len() <= 1 {
            continue;
        }
        let mut next = Vec::with_capacity((out.len() * copies.len()).min(MAX_COPIES_DRAWN));
        'all: for (later, fade) in &copies {
            for (earlier, share) in &out {
                if next.len() >= MAX_COPIES_DRAWN {
                    break 'all;
                }
                next.push((earlier.then(later), share * fade));
            }
        }
        out = next;
    }
    out
}

/// `own`, the box round the item's path, grown to hold what `mods` do to it
/// at `t`, and the margin an offset adds outside that box. A twist keeps the
/// art inside the box it was turned in; a deform moves the outline no further
/// than its amount; an outline pushed out is art outside the path, while one
/// pulled in never needs more room.
pub(super) fn grown(
    mods: &[&ShapeModifier],
    own: (f64, f64, f64, f64),
    path: &BezierPath,
    t: f64,
) -> ((f64, f64, f64, f64), f64) {
    let mut grown = own;
    let mut margin = 0.0;
    for m in mods {
        match &m.kind {
            ModifierKind::Twist(w) => grown = w.bounds(path, t).unwrap_or(own),
            ModifierKind::Deform(d) => grown = d.bounds(grown, own, t),
            ModifierKind::Offset(o) => margin += o.amount.value_at(t).max(0.0),
            _ => {}
        }
    }
    (grown, margin)
}

/// What a 0.9 item carried as modifier fields. Read only; never written.
#[derive(Deserialize)]
struct LegacyMods {
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    trim_start: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    trim_end: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    trim_offset: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    offset_amount: Property,
    #[serde(default = "one", with = "crate::mask::still_or_keyed")]
    repeat_copies: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    repeat_offset: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    repeat_anchor_x: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    repeat_anchor_y: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    repeat_position_x: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    repeat_position_y: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    repeat_rotation: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    repeat_scale: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    repeat_start_opacity: Property,
    #[serde(default = "crate::paint::full", with = "crate::mask::still_or_keyed")]
    repeat_end_opacity: Property,
    #[serde(default)]
    twist: Option<Twist>,
    #[serde(default)]
    deform: Option<Deform>,
    #[serde(default)]
    smooth: Option<Smooth>,
}

fn one() -> Property {
    Property::fixed(1.0)
}

/// An id for an entry lifted out of `item`, the same one every time the file
/// is read: the item's own id, stirred by the entry's kind.
fn lifted_id(item: Uuid, tag: &str) -> Uuid {
    let salt = tag.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    Uuid::from_u128(item.as_u128() ^ (u128::from(salt) << 64 | u128::from(salt)))
}

/// True for a property still at `value`, which an older file left out.
fn still(p: &Property, value: f64) -> bool {
    matches!(p.animation, Animation::Static(v) if v == value) && p.extra.is_empty()
}

impl LegacyMods {
    /// The entries these fields are, in the order they used to apply, each
    /// named for its kind and given an id made from the item's own, so a
    /// file reads the same way every time it is opened.
    fn lifted(self, item: Uuid) -> Vec<ShapeModifier> {
        let mut out = Vec::new();
        let mut push = |kind: ModifierKind| {
            out.push(ShapeModifier {
                id: lifted_id(item, kind.label()),
                name: kind.label().into(),
                kind,
            });
        };
        if let Some(smooth) = self.smooth {
            push(ModifierKind::Smooth(smooth));
        }
        if let Some(twist) = self.twist {
            push(ModifierKind::Twist(twist));
        }
        if let Some(deform) = self.deform {
            push(ModifierKind::Deform(deform));
        }
        if !still(&self.offset_amount, 0.0) {
            push(ModifierKind::Offset(Offset {
                amount: self.offset_amount,
                extra: serde_json::Map::new(),
            }));
        }
        if !(still(&self.trim_start, 0.0)
            && still(&self.trim_end, 100.0)
            && still(&self.trim_offset, 0.0))
        {
            push(ModifierKind::Trim(Trim {
                start: self.trim_start,
                end: self.trim_end,
                offset: self.trim_offset,
                extra: serde_json::Map::new(),
            }));
        }
        if !still(&self.repeat_copies, 1.0) {
            push(ModifierKind::Repeat(Repeater {
                copies: self.repeat_copies,
                offset: self.repeat_offset,
                anchor_x: self.repeat_anchor_x,
                anchor_y: self.repeat_anchor_y,
                position_x: self.repeat_position_x,
                position_y: self.repeat_position_y,
                rotation: self.repeat_rotation,
                scale: self.repeat_scale,
                start_opacity: self.repeat_start_opacity,
                end_opacity: self.repeat_end_opacity,
                extra: serde_json::Map::new(),
            }));
        }
        out
    }
}

/// An item as the file holds it: its modifier fields, if it is old enough to
/// have them, taken off before the item itself reads the rest.
#[derive(Deserialize)]
struct OldItem {
    #[serde(flatten)]
    mods: LegacyMods,
    #[serde(flatten)]
    item: ShapeItem,
}

#[allow(clippy::large_enum_variant)]
#[derive(Deserialize)]
#[serde(untagged)]
enum Wire {
    Modifier(ShapeModifier),
    Group(ShapeGroup),
    Item(OldItem),
}

/// Read a contents list, lifting an older file's per-item modifier fields
/// into entries. Every place a contents list is read goes through this.
pub fn read_entries<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ShapeEntry>, D::Error> {
    Ok(lifted(Vec::<Wire>::deserialize(d)?))
}

/// `wire` as entries. Old items drawn as one run share the first item's
/// modifiers, which go after the run's last member so they act on all of it.
/// A run with modifiers that has art above it is wrapped in a group, named
/// for its first item, so the modifiers reach no further than they did.
fn lifted(wire: Vec<Wire>) -> Vec<ShapeEntry> {
    let mut out: Vec<ShapeEntry> = Vec::with_capacity(wire.len());
    let mut run: Vec<ShapeEntry> = Vec::new();
    let mut run_mods: Vec<ShapeModifier> = Vec::new();
    fn flush(out: &mut Vec<ShapeEntry>, run: &mut Vec<ShapeEntry>, mods: &mut Vec<ShapeModifier>) {
        let mut entries = std::mem::take(run);
        let mods = std::mem::take(mods);
        if entries.is_empty() {
            return;
        }
        if mods.is_empty() || out.is_empty() {
            out.append(&mut entries);
            out.extend(mods.into_iter().map(ShapeEntry::Modifier));
            return;
        }
        let (id, name) = match &entries[0] {
            ShapeEntry::Item(item) => (lifted_id(item.id, "group"), item.name.clone()),
            _ => (Uuid::now_v7(), String::new()),
        };
        entries.extend(mods.into_iter().map(ShapeEntry::Modifier));
        out.push(ShapeEntry::Group(ShapeGroup {
            id,
            name,
            entries,
            extra: serde_json::Map::new(),
        }));
    }
    for entry in wire {
        match entry {
            Wire::Modifier(m) => {
                flush(&mut out, &mut run, &mut run_mods);
                out.push(ShapeEntry::Modifier(m));
            }
            Wire::Group(g) => {
                flush(&mut out, &mut run, &mut run_mods);
                out.push(ShapeEntry::Group(g));
            }
            Wire::Item(OldItem { mods, item }) => {
                // A member of a run lent its path and nothing else, so its
                // own fields never drew and are let go.
                if item.combine == 0 || run.is_empty() {
                    flush(&mut out, &mut run, &mut run_mods);
                    run_mods = mods.lifted(item.id);
                }
                run.push(ShapeEntry::Item(item));
            }
        }
    }
    flush(&mut out, &mut run, &mut run_mods);
    out
}
