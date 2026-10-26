//! A shape layer's contents across the bridge.
//!
//! The engine keeps the contents as a tree: items, modifiers, and groups of
//! either (`lumit_core::shape::modifier`). The frontend gets it as **one flat
//! list**, each entry with how deep it sits, which is the shape of the
//! Timeline's rows and the shape a list edit wants. A group is the entry that
//! opens it; the entries after it that sit one deeper are inside it, and the
//! first entry no deeper than the group closes it. The whole list is written
//! back the same way, as a mask list or a paint list is.

use flutter_rust_bridge::frb;
use lumit_core::shape::modifier::{
    self, ModifierKind, Offset, Repeater, ShapeEntry, ShapeGroup, ShapeModifier, Smooth, Trim,
};
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::api::effect::BridgeScalar;
use crate::api::layer::{
    clamped_property, BridgeShapeDeform, BridgeShapeItem, BridgeShapeTwist, LayerReference,
};
use crate::api::BridgeError;

/// One entry of a shape layer's contents, and how deep it sits: 0 at the top
/// of the layer, one more inside each group.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShapeEntry {
    pub depth: u32,
    pub kind: BridgeShapeEntryKind,
}

/// What an entry is.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeShapeEntryKind {
    Item(BridgeShapeItem),
    Modifier(BridgeShapeModifier),
    Group(BridgeShapeGroup),
}

/// A group: a name over the entries that follow it one deeper. Its
/// modifiers act inside it and nowhere else.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShapeGroup {
    pub id: Uuid,
    pub name: String,
}

/// A modifier entry: it acts on every item above it in its group, in list
/// order, and two of one kind stack.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShapeModifier {
    pub id: Uuid,
    pub name: String,
    pub kind: BridgeShapeModifierKind,
}

/// The modifiers there are. An offset carries its amount in layer pixels
/// and a smooth its reach, so each is the one number.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeShapeModifierKind {
    Repeat(BridgeShapeRepeater),
    Trim(BridgeShapeTrim),
    Offset(BridgeScalar),
    Twist(BridgeShapeTwist),
    Deform(BridgeShapeDeform),
    Smooth(BridgeScalar),
}

/// **The repeater**: how many copies are drawn, which copy the original is
/// (`offset`), and the step each copy is one more of: moved by `position_*`
/// layer pixels, turned by `rotation` degrees and scaled by `scale` per
/// cent, all about `anchor_*`. The copies fade evenly from `start_opacity`
/// to `end_opacity`.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShapeRepeater {
    pub copies: BridgeScalar,
    pub offset: BridgeScalar,
    pub anchor_x: BridgeScalar,
    pub anchor_y: BridgeScalar,
    pub position_x: BridgeScalar,
    pub position_y: BridgeScalar,
    pub rotation: BridgeScalar,
    pub scale: BridgeScalar,
    pub start_opacity: BridgeScalar,
    pub end_opacity: BridgeScalar,
}

/// **Trim paths**: where along the path's own length the art begins and
/// ends, per cent, and how far the pair is slid along it in degrees.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShapeTrim {
    pub start: BridgeScalar,
    pub end: BridgeScalar,
    pub offset: BridgeScalar,
}

impl BridgeShapeModifier {
    #[frb(ignore)]
    fn read_at(m: &ShapeModifier, offset: Rational) -> Self {
        let at = |p| BridgeScalar::read_at(p, offset);
        Self {
            id: m.id,
            name: m.name.clone(),
            kind: match &m.kind {
                ModifierKind::Repeat(r) => BridgeShapeModifierKind::Repeat(BridgeShapeRepeater {
                    copies: at(&r.copies),
                    offset: at(&r.offset),
                    anchor_x: at(&r.anchor_x),
                    anchor_y: at(&r.anchor_y),
                    position_x: at(&r.position_x),
                    position_y: at(&r.position_y),
                    rotation: at(&r.rotation),
                    scale: at(&r.scale),
                    start_opacity: at(&r.start_opacity),
                    end_opacity: at(&r.end_opacity),
                }),
                ModifierKind::Trim(t) => BridgeShapeModifierKind::Trim(BridgeShapeTrim {
                    start: at(&t.start),
                    end: at(&t.end),
                    offset: at(&t.offset),
                }),
                ModifierKind::Offset(o) => BridgeShapeModifierKind::Offset(at(&o.amount)),
                ModifierKind::Twist(w) => BridgeShapeModifierKind::Twist(BridgeShapeTwist {
                    angle: at(&w.angle),
                    rotation: at(&w.rotation),
                    axis: at(&w.axis),
                    centre_x: at(&w.centre_x),
                    centre_y: at(&w.centre_y),
                }),
                ModifierKind::Deform(d) => BridgeShapeModifierKind::Deform(BridgeShapeDeform {
                    mode: d.mode,
                    amount: at(&d.amount),
                    scale: at(&d.scale),
                    evolution: at(&d.evolution),
                    seed: at(&d.seed),
                    detail: at(&d.detail),
                }),
                ModifierKind::Smooth(s) => BridgeShapeModifierKind::Smooth(at(&s.amount)),
            },
        }
    }

    /// The engine's entry this describes, every number held to its range as
    /// an item's are. `previous` is the entry it replaces, whose
    /// forward-compatibility `extra` is kept when the kind is the same.
    #[frb(ignore)]
    fn write(
        &self,
        previous: Option<&ShapeModifier>,
        offset: Rational,
    ) -> Result<ShapeModifier, BridgeError> {
        let held = |p, lo, hi| clamped_property(p, offset, lo, hi);
        let kept = |same: bool, extra: &serde_json::Map<String, serde_json::Value>| {
            if same {
                extra.clone()
            } else {
                serde_json::Map::new()
            }
        };
        let was = previous.map(|p| &p.kind);
        let kind = match &self.kind {
            BridgeShapeModifierKind::Repeat(r) => ModifierKind::Repeat(Repeater {
                // The count is what the engine holds to 1..MAX_COPIES as it
                // draws; the offset is a copy index and may be negative.
                copies: held(&r.copies, 1.0, modifier::MAX_COPIES as f64)?,
                offset: held(
                    &r.offset,
                    -(modifier::MAX_COPIES as f64),
                    modifier::MAX_COPIES as f64,
                )?,
                anchor_x: held(&r.anchor_x, -100_000.0, 100_000.0)?,
                anchor_y: held(&r.anchor_y, -100_000.0, 100_000.0)?,
                position_x: held(&r.position_x, -100_000.0, 100_000.0)?,
                position_y: held(&r.position_y, -100_000.0, 100_000.0)?,
                // Degrees wrap; a scale of zero collapses a copy to nothing
                // and a negative one mirrors it, both of which are drawings.
                rotation: held(&r.rotation, -360_000.0, 360_000.0)?,
                scale: held(&r.scale, -10_000.0, 10_000.0)?,
                start_opacity: held(&r.start_opacity, 0.0, 100.0)?,
                end_opacity: held(&r.end_opacity, 0.0, 100.0)?,
                extra: match was {
                    Some(ModifierKind::Repeat(p)) => kept(true, &p.extra),
                    _ => serde_json::Map::new(),
                },
            }),
            // Per cent of the path's own length, so anything outside 0..100
            // could only ever draw wrongly. The offset is degrees and wraps.
            BridgeShapeModifierKind::Trim(t) => ModifierKind::Trim(Trim {
                start: held(&t.start, 0.0, 100.0)?,
                end: held(&t.end, 0.0, 100.0)?,
                offset: held(&t.offset, -360_000.0, 360_000.0)?,
                extra: match was {
                    Some(ModifierKind::Trim(p)) => kept(true, &p.extra),
                    _ => serde_json::Map::new(),
                },
            }),
            // Layer pixels, out or in: both directions mean something.
            BridgeShapeModifierKind::Offset(amount) => ModifierKind::Offset(Offset {
                amount: held(amount, -100_000.0, 100_000.0)?,
                extra: match was {
                    Some(ModifierKind::Offset(p)) => kept(true, &p.extra),
                    _ => serde_json::Map::new(),
                },
            }),
            BridgeShapeModifierKind::Twist(w) => ModifierKind::Twist(modifier::Twist {
                angle: held(&w.angle, -360_000.0, 360_000.0)?,
                rotation: held(&w.rotation, -360_000.0, 360_000.0)?,
                axis: held(&w.axis, -360_000.0, 360_000.0)?,
                centre_x: held(&w.centre_x, -100_000.0, 100_000.0)?,
                centre_y: held(&w.centre_y, -100_000.0, 100_000.0)?,
                extra: match was {
                    Some(ModifierKind::Twist(p)) => kept(true, &p.extra),
                    _ => serde_json::Map::new(),
                },
            }),
            // Two readings and no third; the feature size and the detail
            // stay above nothing, as a field read nowhere is no field.
            BridgeShapeModifierKind::Deform(d) => ModifierKind::Deform(modifier::Deform {
                mode: if d.mode <= 1 { d.mode } else { 0 },
                amount: held(&d.amount, -100_000.0, 100_000.0)?,
                scale: held(&d.scale, 1.0, 100_000.0)?,
                evolution: held(&d.evolution, -360_000.0, 360_000.0)?,
                seed: held(&d.seed, 0.0, 1_000_000.0)?,
                detail: held(&d.detail, 0.1, 100.0)?,
                extra: match was {
                    Some(ModifierKind::Deform(p)) => kept(true, &p.extra),
                    _ => serde_json::Map::new(),
                },
            }),
            BridgeShapeModifierKind::Smooth(amount) => ModifierKind::Smooth(Smooth {
                amount: held(amount, 0.0, 10_000.0)?,
                extra: match was {
                    Some(ModifierKind::Smooth(p)) => kept(true, &p.extra),
                    _ => serde_json::Map::new(),
                },
            }),
        };
        Ok(ShapeModifier {
            id: self.id,
            name: self.name.clone(),
            kind,
        })
    }
}

/// `entries` as the flat list the frontend holds, keys carried out by the
/// layer's `offset` as every item's are.
#[frb(ignore)]
pub(crate) fn read_entries(entries: &[ShapeEntry], offset: Rational) -> Vec<BridgeShapeEntry> {
    fn flatten(
        entries: &[ShapeEntry],
        depth: u32,
        offset: Rational,
        out: &mut Vec<BridgeShapeEntry>,
    ) {
        for entry in entries {
            let kind = match entry {
                ShapeEntry::Item(item) => {
                    BridgeShapeEntryKind::Item(BridgeShapeItem::read_at(item, offset))
                }
                ShapeEntry::Modifier(m) => {
                    BridgeShapeEntryKind::Modifier(BridgeShapeModifier::read_at(m, offset))
                }
                ShapeEntry::Group(g) => BridgeShapeEntryKind::Group(BridgeShapeGroup {
                    id: g.id,
                    name: g.name.clone(),
                }),
            };
            out.push(BridgeShapeEntry { depth, kind });
            if let ShapeEntry::Group(g) = entry {
                flatten(&g.entries, depth + 1, offset, out);
            }
        }
    }
    let mut out = Vec::new();
    flatten(entries, 0, offset, &mut out);
    out
}

/// Every modifier in `entries`, groups opened.
fn modifiers(entries: &[ShapeEntry]) -> Vec<&ShapeModifier> {
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            ShapeEntry::Modifier(m) => out.push(m),
            ShapeEntry::Group(g) => out.extend(modifiers(&g.entries)),
            ShapeEntry::Item(_) => {}
        }
    }
    out
}

/// Every group in `entries`, the ones inside groups included.
fn groups(entries: &[ShapeEntry]) -> Vec<&ShapeGroup> {
    let mut out = Vec::new();
    for entry in entries {
        if let ShapeEntry::Group(g) = entry {
            out.push(g);
            out.extend(groups(&g.entries));
        }
    }
    out
}

/// The flat list back as the engine's tree, with everything an entry keeps
/// that the bridge does not carry taken from `before` by id: an item's shape
/// keys and `extra`, a modifier's and a group's `extra`. A path of fewer than
/// two vertices is refused, as a mask's is. `at` is the playhead, for an item
/// whose shape is keyed (see `LayerReference::set_shape_contents`).
#[frb(ignore)]
pub(crate) fn write_entries(
    flat: &[BridgeShapeEntry],
    before: &[ShapeEntry],
    offset: Rational,
    at: Option<Rational>,
) -> Result<Vec<ShapeEntry>, BridgeError> {
    let items = modifier::items(before);
    let mods = modifiers(before);
    let was_groups = groups(before);
    let mut root: Vec<ShapeEntry> = Vec::new();
    // The groups still open, innermost last, each with the depth it sits at.
    let mut open: Vec<(ShapeGroup, u32)> = Vec::new();
    fn push(open: &mut [(ShapeGroup, u32)], root: &mut Vec<ShapeEntry>, entry: ShapeEntry) {
        match open.last_mut() {
            Some((group, _)) => group.entries.push(entry),
            None => root.push(entry),
        }
    }
    for entry in flat {
        // An entry no deeper than an open group is outside it.
        while open.last().is_some_and(|(_, depth)| entry.depth <= *depth) {
            if let Some((group, _)) = open.pop() {
                push(&mut open, &mut root, ShapeEntry::Group(group));
            }
        }
        let written = match &entry.kind {
            BridgeShapeEntryKind::Item(item) => {
                if item.vertices.len() < 2 {
                    return Err(BridgeError::EmptyPath);
                }
                ShapeEntry::Item(match items.iter().find(|p| p.id == item.id) {
                    Some(previous) => item.write_item_over(previous, offset, at)?,
                    None => item.write_item(offset)?,
                })
            }
            BridgeShapeEntryKind::Modifier(m) => {
                ShapeEntry::Modifier(m.write(mods.iter().find(|p| p.id == m.id).copied(), offset)?)
            }
            BridgeShapeEntryKind::Group(g) => {
                open.push((
                    ShapeGroup {
                        id: g.id,
                        name: g.name.clone(),
                        entries: Vec::new(),
                        extra: was_groups
                            .iter()
                            .find(|p| p.id == g.id)
                            .map(|p| p.extra.clone())
                            .unwrap_or_default(),
                    },
                    entry.depth,
                ));
                continue;
            }
        };
        push(&mut open, &mut root, written);
    }
    while let Some((group, _)) = open.pop() {
        push(&mut open, &mut root, ShapeEntry::Group(group));
    }
    Ok(root)
}

impl LayerReference {
    /// Make a repeater's copies real: `repeater`'s, or every repeater's on
    /// the layer when `None`. Each copy becomes a shape of its own, placed
    /// where the copy was drawn and faded as it was faded, and the repeater
    /// goes. `time` is composition time, the playhead, where keyed numbers
    /// are read.
    ///
    /// Into this layer, the copies take the repeater's place in the
    /// contents. Into layers, the first copy stays here and each further
    /// copy becomes a shape layer of its own directly below this one, a copy
    /// of this layer holding only the repeated art and what acted on it
    /// after the repeater. Either way one `Batch`, so one undo step.
    #[frb(sync)]
    pub fn expand_shape_copies(
        &self,
        repeater: Option<Uuid>,
        time: crate::api::effect::BridgeRational,
        into_layers: bool,
    ) -> Result<(), BridgeError> {
        use lumit_core::shape::expand;
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Shape { contents: before } = &layer.kind else {
            return Err(BridgeError::NotShape);
        };
        let t = Rational::new(time.num, time.den)
            .map_err(|_| BridgeError::InvalidKeyframes)?
            .checked_sub(layer.start_offset.0)
            .map_err(|_| BridgeError::InvalidKeyframes)?
            .to_f64();
        if !into_layers {
            let after = match repeater {
                Some(id) => expand::expanded(before, id, t).ok_or(BridgeError::InvalidItem)?,
                None => expand::all_expanded(before, t),
            };
            return self.commit_shape_items(&layer, before, after);
        }
        let parts = match repeater {
            Some(id) => expand::split(before, id, t).ok_or(BridgeError::InvalidItem)?,
            None => expand::split_all(before, t),
        };
        let comp = self.composition()?;
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;
        let mut parts = parts.into_iter();
        let kept = parts.next().unwrap_or_default();
        let mut ops = self.shape_ops(&layer, before, kept);
        for (n, part) in parts.enumerate() {
            let mut copy = layer.clone();
            copy.id = Uuid::now_v7();
            copy.name = format!("{} {}", layer.name, n + 2);
            for effect in &mut copy.effects {
                effect.id = Uuid::now_v7();
            }
            // The new layer's picture is its own art's box, so its position
            // follows that box's corner from the one the art was drawn in.
            if let (Some((x0, y0, _, _)), Some((x1, y1, _, _))) = (
                lumit_core::shape::contents_bounds(before, 0.0),
                lumit_core::shape::contents_bounds(&part, 0.0),
            ) {
                for (property, delta) in [
                    (&mut copy.transform.position_x, x1 - x0),
                    (&mut copy.transform.position_y, y1 - y0),
                ] {
                    if let lumit_core::anim::Animation::Static(value) = property.animation {
                        property.animation = lumit_core::anim::Animation::Static(value + delta);
                    }
                }
            }
            copy.kind = lumit_core::model::LayerKind::Shape { contents: part };
            ops.push(lumit_core::Op::AddLayer {
                comp: self.comp_id,
                index: index + n + 1,
                layer: Box::new(copy),
            });
        }
        self.commit(lumit_core::Op::Batch { ops })
    }
}
