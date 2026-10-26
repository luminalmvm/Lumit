//! Making a repeater's copies real.
//!
//! A repeater draws the art above it several times. Expanding it draws
//! those copies out as items of their own, each placed where its copy was
//! drawn and faded as it was faded, and takes the repeater away, so the
//! picture stays and every copy can be edited on its own. Repeaters above
//! it that fed it are expanded first, so a grid comes out as its cells.
//!
//! The copy's transform is baked into the item: its path and its shape keys
//! are moved, its outline and dashes scaled, its gradient aimed where the
//! copy aimed it. The modifiers between the art and the repeater stay, and
//! act on the new items as they acted on the copies. A trim is the same
//! either way; an offset under a scaled copy, a twist and a deform read the
//! new positions, so they can differ a little.

use uuid::Uuid;

use super::geometry::{transform_path, Affine};
use super::modifier::{ModifierKind, Repeater, ShapeEntry, ShapeGroup};
use super::ShapeItem;
use crate::anim::{Animation, Property};

/// `entries` with the repeater `id` expanded at `t`, or `None` when there is
/// no such repeater.
#[must_use]
pub fn expanded(entries: &[ShapeEntry], id: Uuid, t: f64) -> Option<Vec<ShapeEntry>> {
    let mut out = entries.to_vec();
    expand_in(&mut out, id, t).then_some(out)
}

/// `entries` with every repeater expanded, top to bottom.
#[must_use]
pub fn all_expanded(entries: &[ShapeEntry], t: f64) -> Vec<ShapeEntry> {
    let mut out = entries.to_vec();
    while let Some(id) = first_repeater(&out) {
        expand_in(&mut out, id, t);
    }
    out
}

/// The repeater `id` at `t` as one contents list per copy: the first is what
/// the layer keeps, the copy with everything else in the list, and each of
/// the rest holds only the repeated art placed by its copy, with the
/// modifiers that acted after the repeater. `None` when there is no such
/// repeater.
#[must_use]
pub fn split(entries: &[ShapeEntry], id: Uuid, t: f64) -> Option<Vec<Vec<ShapeEntry>>> {
    let mut entries = entries.to_vec();
    let path = path_to(&entries, id)?;
    // Repeaters above it inside its group fed it, and are made real first,
    // so this one is the only one acting on its art.
    while let Some(feeder) = first_repeater(prefix_of(&entries, &path)) {
        expand_in(&mut entries, feeder, t);
    }
    let path = path_to(&entries, id)?;
    let copies = repeater_at(&entries, &path)?.copies_at(t);
    Some(
        copies
            .iter()
            .enumerate()
            .map(|(n, copy)| with_copy(&entries, &path, copy, n, n == 0))
            .collect(),
    )
}

/// Every copy of every repeater as a contents list of its own: the first
/// repeater is split, then each part split again by the next, until none is
/// left. A row of three under a row of two is six.
#[must_use]
pub fn split_all(entries: &[ShapeEntry], t: f64) -> Vec<Vec<ShapeEntry>> {
    match first_repeater(entries).and_then(|id| split(entries, id, t)) {
        Some(parts) => parts.iter().flat_map(|p| split_all(p, t)).collect(),
        None => vec![entries.to_vec()],
    }
}

/// Expand the repeater `id` wherever it sits in `entries`. False when it is
/// not there.
fn expand_in(entries: &mut Vec<ShapeEntry>, id: Uuid, t: f64) -> bool {
    let Some(path) = path_to(entries, id) else {
        return false;
    };
    while let Some(feeder) = first_repeater(prefix_of(entries, &path)) {
        expand_in(entries, feeder, t);
    }
    let Some(path) = path_to(entries, id) else {
        return false;
    };
    let Some(copies) = repeater_at(entries, &path).map(|r| r.copies_at(t)) else {
        return false;
    };
    *entries = with_copies(entries, &path, &copies);
    true
}

/// The indices down the groups to the entry `id`: its place in the top list,
/// then in each group on the way.
fn path_to(entries: &[ShapeEntry], id: Uuid) -> Option<Vec<usize>> {
    for (i, entry) in entries.iter().enumerate() {
        match entry {
            ShapeEntry::Modifier(m) if m.id == id => return Some(vec![i]),
            ShapeEntry::Group(g) => {
                if let Some(mut rest) = path_to(&g.entries, id) {
                    rest.insert(0, i);
                    return Some(rest);
                }
            }
            _ => {}
        }
    }
    None
}

/// The entries above `path`'s end in its own group: what a modifier there
/// acts on.
fn prefix_of<'a>(entries: &'a [ShapeEntry], path: &[usize]) -> &'a [ShapeEntry] {
    match path {
        [last] => &entries[..*last],
        [first, rest @ ..] => match &entries[*first] {
            ShapeEntry::Group(g) => prefix_of(&g.entries, rest),
            _ => &[],
        },
        [] => &[],
    }
}

fn repeater_at<'a>(entries: &'a [ShapeEntry], path: &[usize]) -> Option<&'a Repeater> {
    match path {
        [last] => match entries.get(*last)? {
            ShapeEntry::Modifier(m) => match &m.kind {
                ModifierKind::Repeat(r) => Some(r),
                _ => None,
            },
            _ => None,
        },
        [first, rest @ ..] => match entries.get(*first)? {
            ShapeEntry::Group(g) => repeater_at(&g.entries, rest),
            _ => None,
        },
        [] => None,
    }
}

/// The first repeater in `entries`, groups opened, in list order.
fn first_repeater(entries: &[ShapeEntry]) -> Option<Uuid> {
    entries.iter().find_map(|e| match e {
        ShapeEntry::Modifier(m) if matches!(m.kind, ModifierKind::Repeat(_)) => Some(m.id),
        ShapeEntry::Group(g) => first_repeater(&g.entries),
        _ => None,
    })
}

/// `entries` with the repeater at `path` replaced by its `copies` of the art
/// above it: the art drawn out, the repeater gone, everything else kept.
fn with_copies(
    entries: &[ShapeEntry],
    path: &[usize],
    copies: &[(Affine, f64)],
) -> Vec<ShapeEntry> {
    match path {
        [r] => {
            let mut out = baked(&entries[..*r], copies);
            out.extend(entries[r + 1..].iter().cloned());
            out
        }
        [g, rest @ ..] => entries
            .iter()
            .enumerate()
            .map(|(i, e)| match e {
                ShapeEntry::Group(group) if i == *g => ShapeEntry::Group(ShapeGroup {
                    entries: with_copies(&group.entries, rest, copies),
                    ..group.clone()
                }),
                _ => e.clone(),
            })
            .collect(),
        [] => entries.to_vec(),
    }
}

/// One copy's layer: the art above the repeater at `path` placed by `copy`
/// (the `n`th), the repeater gone. With `whole`, everything else in the
/// list too; without, only the modifiers that act on that art afterwards,
/// which is what a layer of one copy needs.
fn with_copy(
    entries: &[ShapeEntry],
    path: &[usize],
    copy: &(Affine, f64),
    n: usize,
    whole: bool,
) -> Vec<ShapeEntry> {
    let after = |rest: &[ShapeEntry]| -> Vec<ShapeEntry> {
        if whole {
            rest.to_vec()
        } else {
            rest.iter()
                .filter(|e| matches!(e, ShapeEntry::Modifier(_)))
                .cloned()
                .collect()
        }
    };
    match path {
        [r] => {
            let mut out = baked_nth(&entries[..*r], copy, n);
            out.extend(after(&entries[r + 1..]));
            out
        }
        [g, rest @ ..] => {
            let mut out = Vec::new();
            for (i, e) in entries.iter().enumerate() {
                match e {
                    ShapeEntry::Group(group) if i == *g => {
                        out.push(ShapeEntry::Group(ShapeGroup {
                            entries: with_copy(&group.entries, rest, copy, n, whole),
                            ..group.clone()
                        }));
                        out.extend(after(&entries[i + 1..]));
                        break;
                    }
                    _ if whole => out.push(e.clone()),
                    _ => {}
                }
            }
            out
        }
        [] => entries.to_vec(),
    }
}

/// `art` drawn out `copies` times: each run of items becomes its copies,
/// last copy first so the original ends up on top as the repeater drew it;
/// a group's art is drawn out inside the group; a modifier stays where it
/// was, still acting on what is above it.
fn baked(art: &[ShapeEntry], copies: &[(Affine, f64)]) -> Vec<ShapeEntry> {
    let mut out = Vec::with_capacity(art.len() * copies.len());
    let mut i = 0;
    while i < art.len() {
        match &art[i] {
            ShapeEntry::Item(_) => {
                // The run this item starts: it and the items joined onto it.
                let mut end = i + 1;
                while end < art.len()
                    && matches!(&art[end], ShapeEntry::Item(item) if item.combine != 0)
                {
                    end += 1;
                }
                for (n, copy) in copies.iter().enumerate().rev() {
                    for entry in &art[i..end] {
                        if let ShapeEntry::Item(item) = entry {
                            out.push(ShapeEntry::Item(baked_item(item, copy, n)));
                        }
                    }
                }
                i = end;
            }
            ShapeEntry::Group(g) => {
                out.push(ShapeEntry::Group(ShapeGroup {
                    entries: baked(&g.entries, copies),
                    ..g.clone()
                }));
                i += 1;
            }
            ShapeEntry::Modifier(m) => {
                out.push(ShapeEntry::Modifier(m.clone()));
                i += 1;
            }
        }
    }
    out
}

/// `art` placed by one `copy`, the `n`th: every item moved, nothing drawn
/// out more than once.
fn baked_nth(art: &[ShapeEntry], copy: &(Affine, f64), n: usize) -> Vec<ShapeEntry> {
    art.iter()
        .map(|e| match e {
            ShapeEntry::Item(item) => ShapeEntry::Item(baked_item(item, copy, n)),
            ShapeEntry::Group(g) => ShapeEntry::Group(ShapeGroup {
                entries: baked_nth(&g.entries, copy, n),
                ..g.clone()
            }),
            ShapeEntry::Modifier(m) => ShapeEntry::Modifier(m.clone()),
        })
        .collect()
}

/// `item` as the repeater drew its `n`th copy: placed by `m`, faded by
/// `fade`. The first copy keeps the item's own id and name; the rest are
/// new items, numbered. A copy is a scaled **drawing**, so the outline and
/// the dashes scale with it, as they did when it was a copy.
fn baked_item(item: &ShapeItem, (m, fade): &(Affine, f64), n: usize) -> ShapeItem {
    let mut out = item.clone();
    if n > 0 {
        out.id = Uuid::now_v7();
        out.name = format!("{} {}", item.name, n + 1);
    }
    out.path = transform_path(&item.path, m);
    for key in &mut out.path_keys {
        key.path = transform_path(&key.path, m);
    }
    let scale = m.scale();
    scaled(&mut out.stroke_width, scale);
    for dash in &mut out.dashes {
        scaled(dash, scale);
    }
    scaled(&mut out.dash_offset, scale);
    scaled(&mut out.opacity, *fade);
    // The ramp's two points go with the copy, where both stand still; a
    // keyed point has no one place to move.
    for (x, y) in [
        (&mut out.gradient_start_x, &mut out.gradient_start_y),
        (&mut out.gradient_end_x, &mut out.gradient_end_y),
    ] {
        if let (Animation::Static(px), Animation::Static(py)) = (&x.animation, &y.animation) {
            let (nx, ny) = m.apply((*px, *py));
            x.animation = Animation::Static(nx);
            y.animation = Animation::Static(ny);
        }
    }
    out
}

/// `p` multiplied by `k`, every key of it. An expression is left to say its
/// own number.
fn scaled(p: &mut Property, k: f64) {
    match &mut p.animation {
        Animation::Static(v) => *v *= k,
        Animation::Keyframed(keys) => {
            for key in keys {
                key.value *= k;
            }
        }
        Animation::Expression(_) => {}
    }
}
