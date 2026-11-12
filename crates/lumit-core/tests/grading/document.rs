//! A shot's grade kept with the project: versions, grade groups, shared
//! nodes and memories each come through save and open as they went in, and
//! each edit undoes as one step.
//!
//! Driven through [`DocumentStore`], because the journal is half the claim.

use std::sync::Arc;

use lumit_core::fx::effects::grade_group::{is_reference, Part};
use lumit_core::grade::memories::{self, GradeMemory};
use lumit_core::grade::{groups, shared, versions, GradeError};
use lumit_core::model::Document;
use lumit_core::ops::Op;
use lumit_core::store::DocumentStore;

use crate::support::{comp_of, fx, stack_of};

/// Commit `op`, then check one undo puts back exactly the document before it
/// and one redo the one after. Answers the document after.
fn step(store: &DocumentStore, op: Option<Op>) -> Arc<Document> {
    let before = store.snapshot();
    let after = store
        .commit(op.expect("the edit changes something"))
        .unwrap();
    assert_eq!(*store.undo().unwrap().unwrap(), *before, "one undo");
    assert_eq!(*store.redo().unwrap().unwrap(), *after, "one redo");
    after
}

fn names(doc: &Document, shot: lumit_core::grade::Shot) -> Vec<String> {
    stack_of(doc, shot)
        .iter()
        .map(|e| e.effect.match_name.clone())
        .collect()
}

#[test]
fn versions_groups_shared_nodes_and_memories_survive_save_and_undo() {
    let (doc, shots) = comp_of(3, || vec![fx("correction", &[])]);
    let store = DocumentStore::new(doc);
    let [s0, s1, s2] = [shots[0], shots[1], shots[2]];

    // A group of two: each member wears a Pre and a Post round its grade.
    let (g, op) = groups::create(&store.snapshot(), &[s0, s1]).unwrap();
    let doc = step(&store, Some(op));
    assert_eq!(groups::members(&doc, g), vec![s0, s1]);
    assert_eq!(
        names(&doc, s0),
        ["grade_group", "correction", "grade_group"]
    );
    assert_eq!(names(&doc, s2), ["correction"]);

    // The group's Pre list goes where each member's Pre reference stands.
    let exposure = fx("exposure", &[]);
    let op = groups::set_list(&doc, g, Part::Pre, vec![exposure.clone()]).unwrap();
    let doc = step(&store, op);
    let layer = doc.comp(s1.comp).unwrap().layers[1].clone();
    let worn = groups::worn_at(&doc, &layer, 0.0).unwrap();
    let worn: Vec<&str> = worn
        .effects
        .iter()
        .map(|e| e.effect.match_name.as_str())
        .collect();
    assert_eq!(worn, ["exposure", "correction"]);
    assert_eq!(groups::home_of(&doc, exposure.id), Some((g, Part::Pre)));
    let mut preview = (*doc).clone();
    assert!(groups::stage(&mut preview, std::slice::from_ref(&exposure)));
    assert!(!groups::stage(&mut preview, &[fx("blur", &[])]));

    assert_eq!(groups::rename(&doc, g, " "), Err(GradeError::EmptyName));
    let doc = step(&store, groups::rename(&doc, g, "Faces").unwrap());
    assert_eq!(groups::rename(&doc, g, "Faces"), Ok(None));
    assert_eq!(groups::group(&doc, g).unwrap().name, "Faces");

    // The third shot joins, the second leaves.
    let doc = step(&store, groups::set_group(&doc, s2, Some(g)).unwrap());
    let doc = step(&store, groups::set_group(&doc, s1, None).unwrap());
    assert_eq!(groups::members(&doc, g), vec![s0, s2]);
    assert_eq!(names(&doc, s1), ["correction"]);

    // Versions on the second shot: a copy, switched to and back, renamed,
    // and the live one deleted.
    let original = stack_of(&doc, s1)[0].id;
    let (second, op) = versions::add(&doc, s1, true).unwrap();
    let doc = step(&store, op);
    let set = lumit_core::grade::read(&doc, s1)
        .unwrap()
        .1
        .unwrap()
        .clone();
    let first = set.versions[0].id;
    assert_eq!((set.live, set.versions.len()), (second, 2));
    assert_eq!(stack_of(&doc, s1)[0].id, original, "the stack is untouched");
    let doc = step(&store, versions::switch(&doc, s1, first).unwrap());
    assert_ne!(stack_of(&doc, s1)[0].id, original, "the copy is in");
    let doc = step(&store, versions::rename(&doc, s1, second, "Warm").unwrap());
    let doc = step(&store, versions::delete(&doc, s1, first).unwrap());
    let set = lumit_core::grade::read(&doc, s1)
        .unwrap()
        .1
        .unwrap()
        .clone();
    assert_eq!((set.live, set.versions[0].name.as_str()), (second, "Warm"));
    assert_eq!(stack_of(&doc, s1)[0].id, original, "the original is back");

    // The first shot's grade node shared with the second.
    let node = stack_of(&doc, s0)[1].id;
    let (n, op) = shared::share(&doc, s0, node, &[s1]).unwrap();
    let doc = step(&store, Some(op));
    assert_eq!(shared::users(&doc, n), vec![s0, s1]);
    assert_eq!(shared::add_ref(&doc, s1, n, None).unwrap().1, None);
    assert_eq!(
        groups::members(&doc, g),
        vec![s0, s2],
        "sharing isn't a group"
    );
    assert_eq!(
        groups::set_group(&doc, s1, Some(n)),
        Err(GradeError::UnknownGroup)
    );

    // A memory in slot 1; slot 9 doesn't exist, and keeping the same again
    // changes nothing.
    let memory = GradeMemory {
        slot: 0,
        grade: "[]".into(),
        picture: String::new(),
    };
    let doc = step(&store, memories::set(&doc, 1, Some(memory.clone())));
    assert_eq!(doc.grade_memories[0].slot, 1);
    assert!(memories::set(&doc, 9, Some(memory.clone())).is_none());
    assert!(memories::set(&doc, 1, Some(memory)).is_none());

    // Save and open.
    let text = serde_json::to_string(&*doc).unwrap();
    let opened: Document = serde_json::from_str(&text).unwrap();
    assert_eq!(opened, *doc);
    assert_eq!(serde_json::to_string(&opened).unwrap(), text);

    // Unsharing hands each user its own copy; deleting the group takes its
    // references out of every member.
    let doc = step(&store, Some(shared::unshare(&doc, n).unwrap()));
    assert_eq!(shared::nodes(&doc).count(), 0);
    let doc = step(&store, Some(groups::delete(&doc, g).unwrap()));
    assert!(doc.grade_groups.is_empty());
    for s in [s0, s1, s2] {
        assert!(stack_of(&doc, s).iter().all(|e| !is_reference(e)));
        assert!(names(&doc, s).contains(&"correction".to_owned()));
    }
    assert_eq!(groups::delete(&doc, g), Err(GradeError::UnknownGroup));
    let doc = step(&store, memories::set(&doc, 1, None));
    assert!(doc.grade_memories.is_empty());
}
