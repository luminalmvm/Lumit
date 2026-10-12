//! Bringing compositions out of another project and into the open one.
//!
//! The other project is an earlier save of the same one, from its history,
//! or any `.lum`. Each composition asked for arrives as a new one, with what
//! it places: the compositions nested in it, its solids and its footage.
//! Footage the project already has is used again. Everything else is given new
//! ids, so a composition brought back sits beside its own later self and
//! nothing that was there is touched. Where the project already has a
//! composition by its name, it arrives named for where it came from as well,
//! so the two can be told apart.
//!
//! Nothing here changes a document. It works out the ops that would, and one
//! batch of them is one undo step. Any thread.

use crate::ProjectError;
use lumit_core::model::{Document, FootageItem, ProjectItem};
use lumit_core::ops::Op;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use uuid::Uuid;

/// What [`bring`] worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct Brought {
    /// The new composition for each one asked for, in the order asked.
    pub comps: Vec<Uuid>,
    /// What adds them and everything they place.
    pub ops: Vec<Op>,
}

/// The ops that add `comps` of the project `from` to `doc`. `from` is a file
/// that has been opened and had its footage found
/// ([`crate::resolve_all_media`]), so each item says where its file is.
///
/// An id that names no composition in `from` refuses the whole call.
///
/// `source` is what `from` is called: the name of a version in the project's
/// history, or when it was saved, or another project's name. A composition
/// whose name the project already has arrives as "Titles (`source`)", and one
/// whose name is free keeps it. An empty `source` leaves every name alone.
///
/// **Ids are followed through what an item serialises to**, not field by
/// field. A composition names things in a great many places: a layer's
/// source, parent and matte, an effect's layer, clip and mask rows, driver
/// wires, node graph boxes, the composition a Node graph effect applies.
/// Written out, each is the id as a string, and everything that owns an id
/// writes it as `id`. So one walk finds what an item needs, one finds every id
/// it defines, and one puts the new ids in. A reference added to the model
/// later is followed without a change here.
pub fn bring(
    doc: &Document,
    from: &Document,
    comps: &[Uuid],
    source: &str,
) -> Result<Brought, ProjectError> {
    if comps.iter().any(|id| from.comp(*id).is_none()) {
        return Err(ProjectError::UnknownComposition);
    }
    // A folder is where an item is filed, never something a composition needs.
    let theirs: BTreeMap<String, &ProjectItem> = from
        .items
        .iter()
        .filter(|item| !matches!(item, ProjectItem::Folder(_)))
        .map(|item| (item.id().to_string(), item))
        .collect();

    // The compositions asked for, and every item one of them names, however
    // deep the nesting goes. Each is looked at once, so a loop ends.
    let mut wanted: BTreeMap<String, Value> = BTreeMap::new();
    let mut queue: Vec<String> = comps.iter().map(Uuid::to_string).collect();
    while let Some(id) = queue.pop() {
        let Some(item) = theirs.get(&id).filter(|_| !wanted.contains_key(&id)) else {
            continue;
        };
        let value = serde_json::to_value(item)?;
        strings(&value, &mut |text| {
            if theirs.contains_key(text) {
                queue.push(text.to_owned());
            }
        });
        wanted.insert(id, value);
    }

    // Old id to new, as each is written. Footage the project has keeps the id
    // it has there, and is not added again.
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    let mut added: Vec<(&ProjectItem, Value)> = Vec::new();
    for item in &from.items {
        let Some(value) = wanted.remove(&item.id().to_string()) else {
            continue;
        };
        if let ProjectItem::Footage(footage) = item {
            let had = doc.items.iter().find_map(|have| match have {
                ProjectItem::Footage(have) if same_file(have, footage) => Some(have.id),
                _ => None,
            });
            if let Some(had) = had {
                ids.insert(footage.id.to_string(), had.to_string());
                continue;
            }
        }
        defined(&value, &mut ids);
        added.push((item, value));
    }

    // The names compositions have here, and each arrival's as it is given.
    let mut taken: BTreeSet<String> = doc
        .items
        .iter()
        .filter_map(|item| match item {
            ProjectItem::Composition(comp) => Some(comp.name.clone()),
            _ => None,
        })
        .collect();
    let mut ops = Vec::new();
    for (at, (item, mut value)) in added.into_iter().enumerate() {
        renamed(&mut value, &ids);
        let mut new: ProjectItem = serde_json::from_value(value)?;
        // Where a file is on this machine is never written down, so it does
        // not come through the walk.
        if let (ProjectItem::Footage(new), ProjectItem::Footage(old)) = (&mut new, item) {
            new.media.absolute_path.clone_from(&old.media.absolute_path);
        }
        if let ProjectItem::Composition(comp) = &mut new {
            comp.name = arriving(&comp.name, source, &taken);
            taken.insert(comp.name.clone());
        }
        let (id, label) = (new.id(), from.item_label(item.id()));
        ops.push(Op::AddItem {
            index: doc.items.len() + at,
            item: Box::new(new),
        });
        if label != 0 {
            ops.push(Op::SetItemLabel { id, label });
        }
    }
    let comps = comps
        .iter()
        .map(|id| Uuid::parse_str(ids.get(&id.to_string())?).ok())
        .collect::<Option<Vec<Uuid>>>()
        .ok_or(ProjectError::UnknownComposition)?;
    Ok(Brought { comps, ops })
}

/// The name a composition arrives under: its own, unless the project has one
/// by that name, when it says where it came from as well. A second arrival
/// from the same place is numbered.
fn arriving(name: &str, source: &str, taken: &BTreeSet<String>) -> String {
    if source.is_empty() || !taken.contains(name) {
        return name.to_owned();
    }
    let from = format!("{name} ({source})");
    let mut named = from.clone();
    let mut n = 2;
    while taken.contains(&named) {
        named = format!("{from} {n}");
        n += 1;
    }
    named
}

/// Whether two footage items read the same file the same way, so a layer can
/// be pointed at either and show the same picture.
///
/// The file is told the way a relink and a shared project tell it: by its
/// fingerprint, which a moved or renamed copy keeps. Where either item has
/// none, the path is all there is to go by.
fn same_file(a: &FootageItem, b: &FootageItem) -> bool {
    let file = match (&a.media.fingerprint, &b.media.fingerprint) {
        (Some(a), Some(b)) => a.likely_same_content(b),
        _ => {
            !a.media.on_disk().is_empty()
                && Path::new(a.media.on_disk()) == Path::new(b.media.on_disk())
        }
    };
    file && a.sequence == b.sequence
        && a.source_layer == b.source_layer
        && a.colour_space == b.colour_space
}

/// Hand `f` every string in `value`.
fn strings(value: &Value, f: &mut impl FnMut(&str)) {
    match value {
        Value::String(text) => f(text),
        Value::Array(list) => list.iter().for_each(|v| strings(v, f)),
        Value::Object(fields) => fields.values().for_each(|v| strings(v, f)),
        _ => {}
    }
}

/// Give every id `value` defines a new one in `ids`. A thing that has an id
/// writes it as its `id` field, and a field of that name that is not an id,
/// such as an effect parameter's, is left alone.
fn defined(value: &Value, ids: &mut BTreeMap<String, String>) {
    match value {
        Value::Array(list) => list.iter().for_each(|v| defined(v, ids)),
        Value::Object(fields) => {
            for (name, v) in fields {
                match v {
                    Value::String(id) if name == "id" && Uuid::parse_str(id).is_ok() => {
                        ids.entry(id.clone())
                            .or_insert_with(|| Uuid::now_v7().to_string());
                    }
                    _ => defined(v, ids),
                }
            }
        }
        _ => {}
    }
}

/// Write the new id wherever `value` has an old one: as a value, and as a
/// field's name where something is kept by id.
fn renamed(value: &mut Value, ids: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(new) = ids.get(text.as_str()) {
                text.clone_from(new);
            }
        }
        Value::Array(list) => list.iter_mut().for_each(|v| renamed(v, ids)),
        Value::Object(fields) => {
            *fields = std::mem::take(fields)
                .into_iter()
                .map(|(name, mut v)| {
                    renamed(&mut v, ids);
                    (ids.get(&name).cloned().unwrap_or(name), v)
                })
                .collect();
        }
        _ => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fixtures::{stress_document, StressParams};
    use crate::{open, resolve_all_media, save};
    use lumit_core::markers::Marker;
    use lumit_core::model::{
        EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue, Fingerprint,
        LayerKind, LinearColour, MatteChannel, MatteRef, SolidDef,
    };
    use lumit_core::{DocumentStore, Rational};
    use std::collections::BTreeSet;
    use std::fs;

    fn print(hash: &str) -> Option<Fingerprint> {
        Some(Fingerprint {
            size: 9,
            mtime_secs: 0,
            head_tail_hash: hash.into(),
        })
    }

    fn ids_of(doc: &Document, comps: bool) -> Vec<Uuid> {
        let wanted = |item: &&ProjectItem| match item {
            ProjectItem::Composition(_) => comps,
            ProjectItem::Footage(_) => !comps,
            _ => false,
        };
        doc.items
            .iter()
            .filter(wanted)
            .map(ProjectItem::id)
            .collect()
    }

    /// Every id written anywhere in `items`, defined or only named.
    fn written(items: &[ProjectItem]) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        strings(&serde_json::to_value(items).unwrap(), &mut |text| {
            if Uuid::parse_str(text).is_ok() {
                found.insert(text.to_owned());
            }
        });
        found
    }

    /// Every id `items` define.
    fn owned(items: &[ProjectItem]) -> BTreeSet<String> {
        let mut ids = BTreeMap::new();
        defined(&serde_json::to_value(items).unwrap(), &mut ids);
        ids.into_keys().collect()
    }

    /// The other project. Its first composition, Titles, has a layer of each
    /// kind of thing a layer can point at: a nested composition, a solid, the
    /// plate and the sky. The sky's layer has a parent and a matte, and the
    /// last layer has an effect that names another layer. A third composition
    /// is in the file and in nothing.
    fn other() -> Document {
        let mut doc = stress_document(&StressParams::TINY);
        let (comps, footage) = (ids_of(&doc, true), ids_of(&doc, false));
        let (plate, sky) = (footage[2], footage[3]);
        for item in &mut doc.items {
            if let ProjectItem::Footage(f) = item {
                if f.id == plate {
                    f.media.fingerprint = print("plate");
                } else if f.id == sky {
                    f.media.fingerprint = print("sky");
                }
            }
        }
        let solid = SolidDef {
            id: Uuid::now_v7(),
            name: "Grey solid".into(),
            colour: LinearColour::BLACK,
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        };

        let nested = doc.comp_mut(comps[1]).unwrap();
        nested.layers.truncate(2);
        nested.layers[0].kind = LayerKind::Footage { item: sky };
        nested.layers[1].kind = LayerKind::Footage { item: plate };

        let titles = doc.comp_mut(comps[0]).unwrap();
        titles.name = "Titles".into();
        titles.markers = vec![Marker::user(Uuid::now_v7(), Rational::ONE)];
        let layers: Vec<Uuid> = titles.layers.iter().map(|l| l.id).collect();
        let l = &mut titles.layers;
        l[0].kind = LayerKind::Precomp { comp: comps[1] };
        l[1].kind = LayerKind::Solid { def: solid.id };
        l[2].kind = LayerKind::Footage { item: plate };
        l[3].kind = LayerKind::Footage { item: sky };
        l[3].parent = Some(layers[0]);
        l[3].matte = Some(MatteRef {
            layer: layers[1],
            channel: MatteChannel::Alpha,
            inverted: false,
            source: Default::default(),
        });
        l[4].kind = LayerKind::Footage { item: plate };
        l[4].effects = vec![EffectInstance {
            id: Uuid::now_v7(),
            effect: EffectKey {
                namespace: EffectNamespace::Placeholder,
                match_name: "made_up".into(),
                version: 1,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: vec![EffectParam {
                id: "depth".into(),
                value: EffectValue::Layer(Some(layers[2])),
                extra: serde_json::Map::new(),
            }],
            sample_temporally: true,
            custom_name: None,
            linked_pairs: Vec::new(),
            plugin_state: None,
            roto: None,
            extra: serde_json::Map::new(),
        }];

        doc.items.push(ProjectItem::Solid(solid));
        doc.item_labels.insert(comps[0], 3);
        doc
    }

    /// The open project: Titles as it has become, under the ids it always
    /// had, and the plate under an id and a path of its own.
    fn mine(other: &Document) -> Document {
        let (comps, footage) = (ids_of(other, true), ids_of(other, false));
        let Some(ProjectItem::Footage(plate)) = other.item(footage[2]) else {
            panic!("the plate is footage");
        };
        let mut plate = plate.clone();
        plate.id = Uuid::now_v7();
        plate.media.relative_path = "moved/plate.mov".into();
        let mut doc = Document::new();
        doc.items = vec![
            ProjectItem::Footage(plate),
            other.item(comps[0]).unwrap().clone(),
        ];
        doc
    }

    #[test]
    fn a_composition_comes_in_whole_and_names_only_its_own_copies() {
        // The other project as its file gives it: saved, read, its footage
        // found. The sky is on disk where the file says. The plate is not, so
        // it is told by its fingerprint alone.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("earlier.lum");
        save(&other(), &file).unwrap();
        let sky = dir.path().join("footage").join("asset3.mov");
        fs::create_dir_all(dir.path().join("footage")).unwrap();
        fs::write(&sky, b"sky").unwrap();
        let (mut from, _) = open(&file).unwrap();
        resolve_all_media(&mut from, dir.path(), &[]);

        let before = mine(&from);
        let titles = ids_of(&from, true)[0];
        let plate = before.items[0].id();
        let store = DocumentStore::new(before.clone());
        let brought = bring(&before, &from, &[titles], "Before the regrade").unwrap();
        let after = store.commit(Op::Batch { ops: brought.ops }).unwrap();

        // What was there is as it was, and nothing moved but the items and
        // the one colour tag.
        let (had, new) = after.items.split_at(before.items.len());
        assert_eq!(had, before.items);
        let mut expected = before.clone();
        expected.items.clone_from(&after.items);
        expected.item_labels.clone_from(&after.item_labels);
        assert_eq!(*after, expected);
        // The sky, Titles, the composition nested in it and the solid. Not
        // the plate, which the project has, nor the composition in nothing.
        assert_eq!(new.len(), 4);

        // Every id written in what came in is one it defines, or the plate's.
        // None is the other file's, which are also the ids Titles has here.
        let (own, named) = (owned(new), written(new));
        let stray: Vec<&String> = named
            .difference(&own)
            .filter(|id| **id != plate.to_string())
            .collect();
        assert!(stray.is_empty(), "names something outside: {stray:?}");
        assert!(named.is_disjoint(&written(&from.items)));

        // The project has a Titles, so this one says where it came from. The
        // composition nested in it has a name nothing here has, and keeps it.
        let comp = after.comp(brought.comps[0]).unwrap();
        assert_eq!(comp.name, "Titles (Before the regrade)");
        assert_eq!(after.item_label(comp.id), 3);
        assert_eq!(comp.markers.len(), 1);
        let l = &comp.layers;
        let LayerKind::Footage { item: sky_item } = l[3].kind else {
            panic!("the sky's layer is footage");
        };
        let Some(ProjectItem::Footage(sky_footage)) = after.item(sky_item) else {
            panic!("the sky came in");
        };
        assert_eq!(Path::new(&sky_footage.media.absolute_path), sky);
        let LayerKind::Precomp { comp: nested } = l[0].kind else {
            panic!("the first layer is a precomp");
        };
        let nested = after.comp(nested).unwrap();
        let nested_name = &from.comp(ids_of(&from, true)[1]).unwrap().name;
        assert_eq!(&nested.name, nested_name);
        let inner: Vec<&LayerKind> = nested.layers.iter().map(|l| &l.kind).collect();
        assert_eq!(
            inner,
            [
                &LayerKind::Footage { item: sky_item },
                &LayerKind::Footage { item: plate }
            ]
        );
        let LayerKind::Solid { def } = l[1].kind else {
            panic!("the second layer is a solid");
        };
        assert!(after.solid(def).is_some());
        assert_eq!(l[2].kind, LayerKind::Footage { item: plate });
        assert_eq!(l[3].parent, Some(l[0].id));
        assert_eq!(l[3].matte.map(|m| m.layer), Some(l[1].id));
        assert_eq!(l[4].effects[0].layer_ref("depth"), Some(l[2].id));

        // One step back takes all of it away, and one forward brings it back.
        assert_eq!(*store.undo().unwrap().unwrap(), before);
        assert_eq!(store.redo().unwrap().unwrap(), after);

        // A second time is a second copy that shares nothing with the first
        // but footage. The sky is in the project now, so it is not added.
        // Both names are taken by now, and the nested one's too.
        let again = bring(&after, &from, &[titles], "Before the regrade").unwrap();
        assert_ne!(again.comps, brought.comps);
        let twice = store.commit(Op::Batch { ops: again.ops }).unwrap();
        let names: BTreeSet<&str> = twice.items[after.items.len()..]
            .iter()
            .filter_map(|item| match item {
                ProjectItem::Composition(comp) => Some(comp.name.as_str()),
                _ => None,
            })
            .collect();
        let nested_again = format!("{nested_name} (Before the regrade)");
        assert_eq!(
            names,
            BTreeSet::from(["Titles (Before the regrade) 2", nested_again.as_str()])
        );

        // Into a project with no Titles it keeps its name, which is the usual
        // case: it is brought back because it was deleted.
        let alone = bring(&Document::new(), &from, &[titles], "Before the regrade").unwrap();
        let alone = DocumentStore::new(Document::new())
            .commit(Op::Batch { ops: alone.ops })
            .unwrap();
        assert!(alone
            .items
            .iter()
            .any(|item| matches!(item, ProjectItem::Composition(comp) if comp.name == "Titles")));
        let (first, second) = twice.items.split_at(after.items.len());
        assert_eq!(first, after.items);
        assert_eq!(second.len(), 3);
        let (own, named) = (owned(second), written(second));
        assert!(own.is_disjoint(&written(first)));
        let shared: BTreeSet<String> = named.difference(&own).cloned().collect();
        assert_eq!(
            shared,
            BTreeSet::from([plate.to_string(), sky_item.to_string()])
        );
    }

    /// An id the other project has not got, and one that is footage there.
    /// Neither gives any ops, so there is nothing to commit.
    #[test]
    fn an_id_that_names_no_composition_is_refused() {
        let from = other();
        let titles = ids_of(&from, true)[0];
        for id in [Uuid::now_v7(), ids_of(&from, false)[0]] {
            assert!(matches!(
                bring(&Document::new(), &from, &[titles, id], ""),
                Err(ProjectError::UnknownComposition)
            ));
        }
    }
}
