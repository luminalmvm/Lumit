//! Layer groups end to end (K-702): grouping, ungrouping, undo, the refusals,
//! and the promise that a project saved before groups existed re-saves
//! byte-identical.
//!
//! Driven through [`DocumentStore`] rather than `apply` directly, because the
//! journal is half the claim: a group is one undo step, and undoing it must put
//! the group back in the slot it came out of.

// A test asserts by panicking; the engine's no-panic rule is about the engine
// (the same allow every other integration test in the workspace carries).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::group::{drawn_members, group_of, LayerGroup};
use lumit_core::model::{
    BlendMode, Composition, Document, Layer, LayerKind, LinearColour, ProjectItem, Switches,
    TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_core::{DocumentStore, Op, OpError};
use uuid::Uuid;

fn secs(n: i64) -> CompTime {
    CompTime(Rational::new(n, 1).unwrap())
}

fn layer(name: &str) -> Layer {
    Layer {
        id: Uuid::now_v7(),
        name: name.into(),
        kind: LayerKind::Null,
        in_point: secs(0),
        out_point: secs(4),
        start_offset: secs(0),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        markers: Vec::new(),
        volume_db: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: BlendMode::Normal,
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        graph: Default::default(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// A document with one comp holding four Nulls, top to bottom: a, b, c, d.
fn doc_with_four() -> (Document, Uuid, Vec<Uuid>) {
    let layers: Vec<Layer> = ["a", "b", "c", "d"].iter().map(|n| layer(n)).collect();
    let ids: Vec<Uuid> = layers.iter().map(|l| l.id).collect();
    let comp = Composition {
        id: Uuid::now_v7(),
        name: "Comp 1".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(25, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        groups: Vec::new(),
        markers: Vec::new(),
        motion_blur: Default::default(),
        master_volume_db: 0.0,
        beat_grid: None,
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(comp));
    (doc, comp_id, ids)
}

fn group_of_ids(name: &str, members: &[Uuid]) -> Box<LayerGroup> {
    Box::new(LayerGroup {
        id: Uuid::now_v7(),
        name: name.into(),
        label: 0,
        members: members.to_vec(),
    })
}

#[test]
fn grouping_folds_a_run_and_undo_puts_it_back() {
    let (doc, comp, ids) = doc_with_four();
    let store = DocumentStore::new(doc);
    let group = group_of_ids("Lower third", &ids[1..3]);
    let group_id = group.id;

    let after = store
        .commit(Op::GroupLayers {
            comp,
            index: 0,
            group,
        })
        .expect("a run of two groups");
    let c = after.comp(comp).unwrap();
    assert_eq!(c.groups.len(), 1);
    assert_eq!(c.groups[0].name, "Lower third");
    // The stack itself is untouched — that is the whole promise.
    assert_eq!(
        c.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c", "d"]
    );
    let stack: Vec<Uuid> = c.layers.iter().map(|l| l.id).collect();
    assert_eq!(drawn_members(&stack, &c.groups[0]), ids[1..3].to_vec());
    assert_eq!(group_of(&stack, &c.groups, ids[2]), Some(group_id));
    assert_eq!(group_of(&stack, &c.groups, ids[0]), None);

    // One undo step, and it takes the whole grouping.
    let undone = store.undo().unwrap().expect("the grouping is undoable");
    assert!(undone.comp(comp).unwrap().groups.is_empty());

    // Redo puts it back with the same id, name and members.
    let redone = store.redo().unwrap().expect("and redoable");
    assert_eq!(redone.comp(comp).unwrap().groups[0].id, group_id);
    assert_eq!(
        redone.comp(comp).unwrap().groups[0].members,
        ids[1..3].to_vec()
    );
}

#[test]
fn ungrouping_leaves_every_layer_where_it_was_and_undo_restores_the_slot() {
    let (doc, comp, ids) = doc_with_four();
    let store = DocumentStore::new(doc);
    // Two groups, so the *slot* an undo restores into is a real question and
    // not always zero.
    let first = group_of_ids("Plates", &ids[0..1]);
    let second = group_of_ids("Titles", &ids[2..4]);
    let (first_id, second_id) = (first.id, second.id);
    store
        .commit(Op::GroupLayers {
            comp,
            index: 0,
            group: first,
        })
        .unwrap();
    store
        .commit(Op::GroupLayers {
            comp,
            index: 1,
            group: second,
        })
        .unwrap();

    let after = store
        .commit(Op::UngroupLayers {
            comp,
            group: first_id,
        })
        .unwrap();
    let c = after.comp(comp).unwrap();
    assert_eq!(c.groups.len(), 1);
    assert_eq!(c.groups[0].id, second_id);
    assert_eq!(c.layers.len(), 4, "ungrouping deletes nothing");

    let undone = store.undo().unwrap().unwrap();
    let g = &undone.comp(comp).unwrap().groups;
    assert_eq!(
        g.iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![first_id, second_id],
        "the group comes back in the slot it left"
    );
}

#[test]
fn a_scattered_selection_is_refused_and_so_is_a_second_home() {
    let (doc, comp, ids) = doc_with_four();
    let store = DocumentStore::new(doc);

    // a and c, with an ungrouped b between them: not a band anything can draw.
    assert_eq!(
        store.commit(Op::GroupLayers {
            comp,
            index: 0,
            group: group_of_ids("Scattered", &[ids[0], ids[2]]),
        }),
        Err(OpError::InvalidGroup)
    );
    assert!(store
        .commit(Op::GroupLayers {
            comp,
            index: 0,
            group: group_of_ids("Empty", &[]),
        })
        .is_err());

    store
        .commit(Op::GroupLayers {
            comp,
            index: 0,
            group: group_of_ids("Titles", &ids[0..2]),
        })
        .unwrap();
    // b is spoken for; a group may not take it as well.
    assert_eq!(
        store.commit(Op::GroupLayers {
            comp,
            index: 1,
            group: group_of_ids("Also titles", &ids[1..3]),
        }),
        Err(OpError::InvalidGroup)
    );
    assert_eq!(
        store.commit(Op::SetGroupName {
            comp,
            group: Uuid::now_v7(),
            name: "nobody".into(),
        }),
        Err(OpError::UnknownGroup)
    );
}

#[test]
fn a_name_and_a_colour_are_one_undo_step_each() {
    let (doc, comp, ids) = doc_with_four();
    let store = DocumentStore::new(doc);
    let group = group_of_ids("Group 1", &ids[0..2]);
    let group_id = group.id;
    store
        .commit(Op::GroupLayers {
            comp,
            index: 0,
            group,
        })
        .unwrap();

    let named = store
        .commit(Op::SetGroupName {
            comp,
            group: group_id,
            name: "Background".into(),
        })
        .unwrap();
    assert_eq!(named.comp(comp).unwrap().groups[0].name, "Background");
    let coloured = store
        .commit(Op::SetGroupLabel {
            comp,
            group: group_id,
            label: 5,
        })
        .unwrap();
    assert_eq!(coloured.comp(comp).unwrap().groups[0].label, 5);

    store.undo().unwrap();
    let back = store.undo().unwrap().unwrap();
    assert_eq!(back.comp(comp).unwrap().groups[0].name, "Group 1");
    assert_eq!(back.comp(comp).unwrap().groups[0].label, 0);
}

/// docs/10 §1.1: a project written before groups existed must re-save with the
/// bytes it arrived with. `groups` is skipped while empty, so an ungrouped comp
/// gains no line for it — and the flattened `extra` bag must not swallow one
/// either.
#[test]
fn a_project_with_no_groups_round_trips_byte_identical() {
    let (doc, _, _) = doc_with_four();
    let before = serde_json::to_string(&doc).unwrap();
    assert!(
        !before.contains("groups"),
        "an ungrouped comp writes no groups key: {before}"
    );
    let reloaded: Document = serde_json::from_str(&before).unwrap();
    assert_eq!(serde_json::to_string(&reloaded).unwrap(), before);
    assert!(reloaded
        .comp(
            reloaded
                .items
                .iter()
                .find_map(|i| match i {
                    ProjectItem::Composition(c) => Some(c.id),
                    _ => None,
                })
                .unwrap()
        )
        .unwrap()
        .groups
        .is_empty());
}

/// A grouped project round-trips too, and a member deleted out from under a
/// group is read as one fewer member rather than as a fault (the same
/// degrade-not-error rule a dangling matte or parent follows).
#[test]
fn a_deleted_member_quietly_leaves_the_group() {
    let (doc, comp, ids) = doc_with_four();
    let store = DocumentStore::new(doc);
    store
        .commit(Op::GroupLayers {
            comp,
            index: 0,
            group: group_of_ids("Titles", &ids[1..3]),
        })
        .unwrap();
    let after = store
        .commit(Op::RemoveLayer {
            comp,
            layer: ids[1],
        })
        .unwrap();
    let c = after.comp(comp).unwrap();
    let stack: Vec<Uuid> = c.layers.iter().map(|l| l.id).collect();
    assert_eq!(
        drawn_members(&stack, &c.groups[0]),
        vec![ids[2]],
        "the group is one short, not broken"
    );

    let json = serde_json::to_string(&*after).unwrap();
    let reloaded: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&reloaded).unwrap(), json);
}
