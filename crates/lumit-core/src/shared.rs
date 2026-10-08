//! What a shared project needs from the document: which parts of it an edit
//! touched, and how two sets of edits made apart come back together.
//!
//! Pure functions over documents and ops. Any thread.

use crate::model::{Composition, Document, ProjectItem};
use crate::ops::{apply, Op, OpError};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use uuid::Uuid;

/// One part of the document an edit touched: the entity, and a path inside it
/// such as `effects/<id>/params/radius`. An empty path is the whole entity, as
/// when it was added or removed. Project-wide settings are keyed on the nil id.
pub type Key = (Uuid, String);

/// The path segment that stands for the order of a list's entries.
const ORDER: &str = "#order";

/// Whether the part `outer` names holds the part `inner` names.
fn holds(outer: &str, inner: &str) -> bool {
    outer.is_empty()
        || inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Whether two keys name the same part of the document, or one holds the
/// other.
#[must_use]
pub fn overlaps(a: &Key, b: &Key) -> bool {
    a.0 == b.0 && (holds(&a.1, &b.1) || holds(&b.1, &a.1))
}

/// Whether any key of `a` overlaps any key of `b`.
#[must_use]
pub fn any_overlap<'a>(
    a: impl IntoIterator<Item = &'a Key>,
    b: impl IntoIterator<Item = &'a Key> + Clone,
) -> bool {
    a.into_iter()
        .any(|x| b.clone().into_iter().any(|y| overlaps(x, y)))
}

/// The parts of the document that differ between `before` and `after`.
///
/// Worked out from the two documents rather than from the op that made the
/// change, so it needs no arm per op and cannot fall behind when one is added.
/// Only what a peer would be sent counts: a footage item's path on this
/// machine, its proxy, the cache folder and the panel layout are left out.
#[must_use]
pub fn footprint(before: &Document, after: &Document) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    let project = Uuid::nil();
    diff_list(
        &before.items,
        &after.items,
        ProjectItem::id,
        (project, "items"),
        &mut keys,
        whole_item,
        item_keys,
    );
    for id in before.item_labels.keys().chain(after.item_labels.keys()) {
        if before.item_labels.get(id) != after.item_labels.get(id) {
            keys.insert((*id, "label".into()));
        }
    }
    macro_rules! settings {
        ($($field:ident),+) => {$(
            if before.$field != after.$field {
                keys.insert((project, stringify!($field).into()));
            }
        )+};
    }
    settings!(
        auto_folders,
        auto_pack,
        anti_aliasing,
        colour_depth,
        colour,
        swatches,
        extra
    );
    keys
}

/// Diff two lists of entities by id. Added and removed entries are keyed
/// whole, changed ones go to `changed`, and a different order among the ones
/// in both is keyed on `order`.
fn diff_list<T: PartialEq>(
    a: &[T],
    b: &[T],
    id: impl Fn(&T) -> Uuid,
    order: (Uuid, &str),
    keys: &mut BTreeSet<Key>,
    whole: impl Fn(&T, &mut BTreeSet<Key>),
    changed: impl Fn(&T, &T, &mut BTreeSet<Key>),
) {
    if a == b {
        return;
    }
    let old: HashMap<Uuid, &T> = a.iter().map(|t| (id(t), t)).collect();
    let new: HashMap<Uuid, &T> = b.iter().map(|t| (id(t), t)).collect();
    for t in a.iter().filter(|t| !new.contains_key(&id(t))) {
        whole(t, keys);
    }
    for t in b {
        match old.get(&id(t)) {
            None => whole(t, keys),
            Some(was) if *was != t => changed(was, t, keys),
            Some(_) => {}
        }
    }
    let kept_before = a.iter().map(&id).filter(|i| new.contains_key(i));
    let kept_after = b.iter().map(&id).filter(|i| old.contains_key(i));
    if !kept_before.eq(kept_after) {
        keys.insert((order.0, order.1.into()));
    }
}

/// An item that came or went takes its layers with it, so an edit to one of
/// them is seen to collide with the removal.
fn whole_item(item: &ProjectItem, keys: &mut BTreeSet<Key>) {
    keys.insert((item.id(), String::new()));
    if let ProjectItem::Composition(comp) = item {
        keys.extend(comp.layers.iter().map(|l| (l.id, String::new())));
    }
}

fn item_keys(a: &ProjectItem, b: &ProjectItem, keys: &mut BTreeSet<Key>) {
    match (a, b) {
        (ProjectItem::Composition(a), ProjectItem::Composition(b)) => comp_keys(a, b, keys),
        _ => value_keys(a.id(), a, b, keys),
    }
}

fn comp_keys(a: &Composition, b: &Composition, keys: &mut BTreeSet<Key>) {
    let mut found = BTreeSet::new();
    diff_list(
        &a.layers,
        &b.layers,
        |l| l.id,
        (a.id, "layers"),
        &mut found,
        |l, keys| {
            keys.insert((l.id, String::new()));
        },
        |x, y, keys| value_keys(x.id, x, y, keys),
    );
    macro_rules! fields {
        ($($field:ident),+) => {$(
            if a.$field != b.$field {
                let mut path = String::from(stringify!($field));
                match (serde_json::to_value(&a.$field), serde_json::to_value(&b.$field)) {
                    (Ok(x), Ok(y)) => walk(a.id, &x, &y, &mut path, &mut found),
                    _ => {
                        found.insert((a.id, path));
                    }
                }
            }
        )+};
    }
    fields!(
        name,
        width,
        height,
        frame_rate,
        duration,
        background,
        work_area,
        groups,
        markers,
        motion_blur,
        master_volume_db,
        sound_mix,
        beat_grid,
        graph,
        extra
    );
    // A field added to the composition since this list was written.
    if found.is_empty() {
        found.insert((a.id, String::new()));
    }
    keys.extend(found);
}

/// Key the parts that differ between two values of one entity, by what they
/// serialise to. A difference that never reaches the file, such as where a
/// footage item was found on this machine, yields nothing.
fn value_keys<T: serde::Serialize>(id: Uuid, a: &T, b: &T, keys: &mut BTreeSet<Key>) {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => walk(id, &a, &b, &mut String::new(), keys),
        _ => {
            keys.insert((id, String::new()));
        }
    }
}

/// The ids of a list's entries, when it is a list of things with ids: each an
/// object with an `id`, or each an id itself, and no two alike. An effect
/// stack, an effect's parameters, a mask list and a folder's children all are.
/// A list of keyframes is not, and is compared and replaced whole.
fn ids(list: &[Value]) -> Option<Vec<&str>> {
    let ids = list.iter().map(|entry| match entry {
        Value::Object(fields) => fields.get("id")?.as_str(),
        Value::String(id) => Uuid::try_parse(id).is_ok().then_some(id.as_str()),
        _ => None,
    });
    let ids: Vec<&str> = ids.collect::<Option<_>>()?;
    let distinct: HashSet<&str> = ids.iter().copied().collect();
    (distinct.len() == ids.len()).then_some(ids)
}

/// Whether two objects are different variants of one enum, which serialise as
/// one field each, named for the variant. Those are compared whole: a field
/// from each would be neither.
fn variants(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    a.len() == 1 && b.len() == 1 && a.keys().ne(b.keys())
}

/// Whether the ids both lists hold come in a different order in each.
fn reordered(a: &[&str], b: &[&str]) -> bool {
    let kept_a = a.iter().filter(|id| b.contains(id));
    let kept_b = b.iter().filter(|id| a.contains(id));
    kept_a.ne(kept_b)
}

/// Run `f` with `segment` on the end of `path`. An empty one is `path` itself.
fn within(path: &mut String, segment: &str, f: impl FnOnce(&mut String)) {
    let len = path.len();
    if len > 0 && !segment.is_empty() {
        path.push('/');
    }
    path.push_str(segment);
    f(path);
    path.truncate(len);
}

/// Key what differs between `a` and `b`, as finely as [`merge`] can put two
/// people's changes back together: a field of an object, an entry of a list
/// of things with ids, and the order of such a list.
fn walk(id: Uuid, a: &Value, b: &Value, path: &mut String, keys: &mut BTreeSet<Key>) {
    if a == b {
        return;
    }
    // The part at `segment` differs as a whole when either side lacks it.
    let mut part = |path: &mut String, segment: &str, x: Option<&Value>, y: Option<&Value>| {
        within(path, segment, |p| match (x, y) {
            (Some(x), Some(y)) => walk(id, x, y, p, keys),
            _ => {
                keys.insert((id, p.clone()));
            }
        });
    };
    match (a, b) {
        (Value::Object(a), Value::Object(b)) if !variants(a, b) => {
            let names = a.keys().chain(b.keys().filter(|k| !a.contains_key(*k)));
            for name in names {
                part(path, name, a.get(name), b.get(name));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            let (Some(a_ids), Some(b_ids)) = (ids(a), ids(b)) else {
                part(path, "", None, None);
                return;
            };
            let old: HashMap<&str, &Value> = a_ids.iter().copied().zip(a).collect();
            for gone in a_ids.iter().filter(|i| !b_ids.contains(i)) {
                part(path, gone, None, None);
            }
            for (entry, now) in b_ids.iter().zip(b) {
                part(path, entry, old.get(entry).copied(), Some(now));
            }
            if reordered(&a_ids, &b_ids) {
                part(path, ORDER, None, None);
            }
        }
        _ => part(path, "", None, None),
    }
}

/// What a value becomes when two people changed it from `was`: `mine` is the
/// edit being applied and `theirs` is what the document holds now. A part
/// only one of them changed keeps that change. A part both changed takes
/// mine. `None` is a field that is not there.
///
/// It goes as deep as [`walk`] does and no deeper, so two edits whose
/// footprints do not overlap lose nothing to each other here.
fn merge(was: Option<&Value>, mine: Option<&Value>, theirs: Option<&Value>) -> Option<Value> {
    if mine == was {
        return theirs.cloned();
    }
    if theirs == was || theirs == mine {
        return mine.cloned();
    }
    match (was, mine, theirs) {
        (Some(Value::Object(w)), Some(Value::Object(m)), Some(Value::Object(t)))
            if !(variants(w, m) || variants(w, t) || variants(m, t)) =>
        {
            let names = m.keys().chain(t.keys().filter(|k| !m.contains_key(*k)));
            let merged = names.filter_map(|name| {
                let value = merge(w.get(name), m.get(name), t.get(name))?;
                Some((name.clone(), value))
            });
            Some(Value::Object(merged.collect()))
        }
        (Some(Value::Array(w)), Some(Value::Array(m)), Some(Value::Array(t))) => {
            match (ids(w), ids(m), ids(t)) {
                (Some(w_ids), Some(m_ids), Some(t_ids)) => Some(Value::Array(merge_lists(
                    (w, &w_ids),
                    (m, &m_ids),
                    (t, &t_ids),
                ))),
                _ => mine.cloned(),
            }
        }
        _ => mine.cloned(),
    }
}

/// [`merge`] for a list of things with ids. An entry either side removed
/// stays removed, one either side added is kept, and one both still hold is
/// merged. The order is mine if I changed it, and otherwise theirs.
fn merge_lists(
    (was, was_ids): (&[Value], &[&str]),
    (mine, mine_ids): (&[Value], &[&str]),
    (theirs, theirs_ids): (&[Value], &[&str]),
) -> Vec<Value> {
    fn by_id<'a>(list: &'a [Value], ids: &[&'a str]) -> HashMap<&'a str, &'a Value> {
        ids.iter().copied().zip(list).collect()
    }
    let (w, m, t) = (
        by_id(was, was_ids),
        by_id(mine, mine_ids),
        by_id(theirs, theirs_ids),
    );
    let (lead, other, added) = if reordered(was_ids, mine_ids) {
        (mine_ids, theirs_ids, theirs)
    } else {
        (theirs_ids, mine_ids, mine)
    };
    let mut merged: Vec<(&str, Value)> = Vec::with_capacity(lead.len());
    for id in lead {
        // In both to start with, and the other side has taken it out.
        if w.contains_key(id) && !other.contains(id) {
            continue;
        }
        let (was, mine, theirs) = (w.get(id), m.get(id), t.get(id));
        if let Some(entry) = merge(was.copied(), mine.copied(), theirs.copied()) {
            merged.push((id, entry));
        }
    }
    // What the other side added goes after whatever it followed there.
    for (at, (id, entry)) in other.iter().zip(added).enumerate() {
        if w.contains_key(id) || lead.contains(id) {
            continue;
        }
        let mut before = other.iter().take(at).rev();
        let after = before.find_map(|prev| merged.iter().position(|(i, _)| i == prev));
        merged.insert(after.map_or(0, |i| i + 1), (id, entry.clone()));
    }
    merged.into_iter().map(|(_, entry)| entry).collect()
}

/// `mine` cut down to what its author changed. They made it where the same
/// values read `was`, and here they read `now`. `None` when there is nothing
/// to cut: the three are not one kind of op, or nobody else's change is in
/// the way.
fn merged_op(was: &Op, mine: &Op, now: &Op) -> Option<Op> {
    let value = |op| serde_json::to_value(op).ok();
    let (was, mine, now) = (value(was)?, value(mine)?, value(now)?);
    let kind = mine.get("op_type")?;
    if was.get("op_type") != Some(kind) || now.get("op_type") != Some(kind) {
        return None;
    }
    let merged = merge(Some(&was), Some(&mine), Some(&now))?;
    if merged == mine {
        return None;
    }
    serde_json::from_value(merged).ok()
}

/// Apply `op`, with an insert moved to the end of its list when the index it
/// was made for no longer exists. Answers the op as applied, and its inverse.
///
/// An add of something already here is applied as nothing. That is an edit
/// reaching a document that took it once already: one the host applied just
/// as its guest was cut off, or one read back from a log. Applied again it
/// would be a second layer with the same id.
fn fit(doc: &mut Document, op: &Op) -> Result<(Op, Op), OpError> {
    let here = match op {
        Op::AddItem { item, .. } => doc.item(item.id()).is_some(),
        Op::AddLayer { comp, layer, .. } => doc
            .comp(*comp)
            .is_some_and(|c| c.layers.iter().any(|l| l.id == layer.id)),
        _ => false,
    };
    if here {
        let nothing = || Op::Batch { ops: Vec::new() };
        return Ok((nothing(), nothing()));
    }
    match apply(doc, op) {
        Ok(inverse) => Ok((op.clone(), inverse)),
        Err(OpError::BadIndex) => {
            let moved = match op {
                Op::AddItem { item, .. } => Op::AddItem {
                    index: doc.items.len(),
                    item: item.clone(),
                },
                Op::AddLayer { comp, layer, .. } => Op::AddLayer {
                    comp: *comp,
                    index: doc.comp(*comp).map_or(0, |c| c.layers.len()),
                    layer: layer.clone(),
                },
                _ => return Err(OpError::BadIndex),
            };
            let inverse = apply(doc, &moved)?;
            Ok((moved, inverse))
        }
        Err(e) => Err(e),
    }
}

/// Apply an edit to a document other people may have changed since it was
/// made. Answers the op as it was applied, and its inverse. The document is
/// untouched when it fails.
///
/// **What `was` is for.** An op writes a whole value: one slider on one
/// effect writes the layer's whole effect stack. Applied as it stands to a
/// document where someone else has since changed another effect on that
/// layer, it would put their effect back as its author last saw it. `was` is
/// what the op replaced where it was made, which is its inverse there. Where
/// the document no longer reads `was`, the op is cut down to the parts its
/// author changed ([`merge`]) and everything else is left as it is here.
/// Without `was` the op is applied whole.
///
/// **An insert past the end.** A layer added at the bottom of a stack someone
/// else has since shortened names a place that no longer exists. Refusing it
/// would lose the layer and every edit made to it, so it lands at the end.
pub fn land(doc: &mut Document, op: &Op, was: Option<&Op>) -> Result<(Op, Op), OpError> {
    if let Op::Batch { ops } = op {
        // A batch's inverse runs backwards, so its last member answers the
        // first.
        let mut olds = match was {
            Some(Op::Batch { ops: was }) if was.len() == ops.len() => Some(was.iter().rev()),
            _ => None,
        };
        let mut scratch = doc.clone();
        let mut landed = Vec::with_capacity(ops.len());
        let mut inverses = Vec::with_capacity(ops.len());
        for member in ops {
            let old = olds.as_mut().and_then(Iterator::next);
            let (op, inverse) = land(&mut scratch, member, old)?;
            landed.push(op);
            inverses.push(inverse);
        }
        inverses.reverse();
        *doc = scratch;
        return Ok((Op::Batch { ops: landed }, Op::Batch { ops: inverses }));
    }
    let (applied, now) = fit(doc, op)?;
    let merged = was
        .filter(|was| **was != now)
        .and_then(|was| merged_op(was, &applied, &now));
    let Some(merged) = merged else {
        return Ok((applied, now));
    };
    // Put back what was here, and apply the cut-down edit in its place.
    if apply(doc, &now).is_err() {
        return Ok((applied, now));
    }
    match apply(doc, &merged) {
        Ok(inverse) => Ok((merged, inverse)),
        // Two changes that are each fine and together are not, such as a wire
        // to a node the other person deleted. The edit stands as it was made.
        Err(_) => {
            let now = apply(doc, &applied)?;
            Ok((applied, now))
        }
    }
}

/// Edits made apart that touch the same part of the document as someone
/// else's. Held back until the person who made them chooses.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    /// What both sides touched, and anything else these ops touch.
    pub keys: BTreeSet<Key>,
    /// The held edits in the order they were made, each with what it
    /// replaced where it was made, which is what [`land`] is given.
    pub ops: Vec<(Op, Op)>,
}

/// The outcome of [`plan_merge`].
#[derive(Debug, Clone, PartialEq)]
pub struct Merge {
    /// Their document with every clean edit of mine applied.
    pub merged: Document,
    /// My edits that touched nothing of theirs, as applied, with inverses.
    pub clean: Vec<(Op, Op)>,
    /// My edits that touched what they touched. Not applied.
    pub conflicts: Vec<Conflict>,
    /// How many of my edits no longer apply at all, such as an edit to a layer
    /// they deleted.
    pub refused: usize,
}

/// Bring `mine`, the edits made since `base`, onto `theirs`, the document the
/// other side reached from the same `base`. Each of mine comes with what it
/// replaced when it was made.
///
/// Each of my edits is landed on their document in order ([`land`]), so it
/// changes only what I changed. One that touches nothing they changed is
/// applied. One that touches something they changed, or something an earlier
/// held edit touched, is held as a [`Conflict`], so nothing of theirs is
/// overwritten without being asked. An edit that changes nothing, because
/// both sides made it, is dropped.
///
/// **An edit that builds on a held one.** I delete a layer they changed and
/// then undo the delete: the delete is held, so on their document the undo
/// finds the layer still there and changes nothing. Judged there it would be
/// dropped, and keeping mine would then delete the layer. So once anything is
/// held, each later edit is also landed on their document with the held
/// edits on it, which is the one it was made on top of, and held with
/// whichever of them it touches.
#[must_use]
pub fn plan_merge(base: &Document, theirs: Document, mine: &[(Op, Op)]) -> Merge {
    let their_keys = footprint(base, &theirs);
    let mut merge = Merge {
        merged: theirs,
        clean: Vec::new(),
        conflicts: Vec::new(),
        refused: 0,
    };
    // Their document with every edit of mine so far on it, held or not.
    // `None` until the first is held: until then it is `merge.merged`.
    let mut meant: Option<Document> = None;
    for (op, was) in mine {
        if let Some(meant) = meant.as_mut() {
            let mut after = meant.clone();
            if land(&mut after, op, Some(was)).is_ok() {
                let keys = footprint(meant, &after);
                *meant = after;
                let held = merge
                    .conflicts
                    .iter_mut()
                    .find(|c| any_overlap(&keys, &c.keys));
                if let Some(conflict) = held {
                    conflict.keys.extend(keys);
                    conflict.ops.push((op.clone(), was.clone()));
                    continue;
                }
            }
        }
        let mut trial = merge.merged.clone();
        let Ok(landed) = land(&mut trial, op, Some(was)) else {
            merge.refused += 1;
            continue;
        };
        let keys = footprint(&merge.merged, &trial);
        if keys.is_empty() {
            continue;
        }
        let held = merge
            .conflicts
            .iter_mut()
            .find(|c| any_overlap(&keys, &c.keys));
        match held {
            Some(conflict) => {
                conflict.keys.extend(keys);
                conflict.ops.push((op.clone(), was.clone()));
            }
            None if any_overlap(&keys, &their_keys) => {
                if meant.is_none() {
                    let mut with = merge.merged.clone();
                    let _ = land(&mut with, op, Some(was));
                    meant = Some(with);
                }
                merge.conflicts.push(Conflict {
                    keys,
                    ops: vec![(op.clone(), was.clone())],
                });
            }
            None => {
                merge.clean.push(landed);
                merge.merged = trial;
            }
        }
    }
    merge
}
