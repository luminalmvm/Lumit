//! A shared project: edits from more than one person converge.

use super::*;

/// What a tap was told.
enum Heard {
    /// An edit made at this end: its number, the edit, and what it replaced.
    Local(u64, Op, Box<Op>),
    /// Someone else's as it was applied, and whether that is how it was sent.
    Remote(RemoteTag, Op, bool),
}

/// One end of a shared project: a store, and what its tap was told.
struct End {
    store: DocumentStore,
    heard: Arc<Mutex<Vec<Heard>>>,
}

fn end(doc: &Document, guest: bool) -> End {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let store = DocumentStore::new(doc.clone());
    let sink = heard.clone();
    store.share(
        guest,
        Arc::new(move |moved| {
            sink.lock().push(match moved {
                Moved::Local { id, op, was } => Heard::Local(id, op.clone(), Box::new(was.clone())),
                Moved::Remote {
                    tag, op, as_sent, ..
                } => Heard::Remote(tag, op.clone(), as_sent),
            });
        }),
    );
    End { store, heard }
}

/// What the host sends one guest.
enum Wire {
    Applied(RemoteTag, Box<Op>),
    Accepted(u64),
    Rejected(u64),
}

/// Carry edits the way sharing does: the host's own first, then each
/// guest's in the order given, every one answered to every guest.
fn deliver(host: &End, guests: &[&End], order: &[usize]) {
    let mut wires: Vec<Vec<Wire>> = guests.iter().map(|_| Vec::new()).collect();
    let broadcast = |wires: &mut Vec<Vec<Wire>>| {
        for heard in host.heard.lock().drain(..) {
            let (tag, op, as_sent) = match heard {
                Heard::Local(id, op, _) => (RemoteTag { peer: 0, id }, op, false),
                Heard::Remote(tag, op, as_sent) => (tag, op, as_sent),
            };
            for (g, wire) in wires.iter_mut().enumerate() {
                // Its author is not sent an edit back that went in as sent.
                wire.push(if as_sent && tag.peer == g as u32 + 1 {
                    Wire::Accepted(tag.id)
                } else {
                    Wire::Applied(tag, Box::new(op.clone()))
                });
            }
        }
    };
    broadcast(&mut wires);
    for &g in order {
        let sent: Vec<_> = guests[g].heard.lock().drain(..).collect();
        for heard in sent {
            let Heard::Local(id, op, was) = heard else {
                continue;
            };
            let tag = RemoteTag {
                peer: g as u32 + 1,
                id,
            };
            match host.store.commit_remote(&op, Some(&was), tag) {
                Ok(()) => broadcast(&mut wires),
                Err(_) => wires[g].push(Wire::Rejected(id)),
            }
        }
    }
    for (g, wire) in wires.into_iter().enumerate() {
        let guest = &guests[g].store;
        for message in wire {
            match message {
                Wire::Applied(tag, op) if tag.peer == g as u32 + 1 => {
                    guest.commit_answer(tag.id, &op).unwrap();
                }
                Wire::Applied(tag, op) => guest.commit_remote(&op, None, tag).unwrap(),
                Wire::Accepted(id) => assert!(guest.acknowledge(id), "in step with the host"),
                Wire::Rejected(id) => guest.reject(id).unwrap(),
            }
        }
        guests[g].heard.lock().clear();
    }
}

fn rename(id: Uuid, name: &str) -> Op {
    Op::RenameItem {
        id,
        name: name.into(),
    }
}

fn name_of(store: &DocumentStore, id: Uuid) -> Option<String> {
    match store.snapshot().item(id)? {
        ProjectItem::Solid(s) => Some(s.name.clone()),
        _ => None,
    }
}

/// Two solids to edit, in a document every end starts from.
fn two_solids() -> (Document, Uuid, Uuid) {
    let seed = DocumentStore::new(Document::new());
    let (x, y) = (loose_item(&seed), loose_item(&seed));
    (Document::clone(&seed.snapshot()), x, y)
}

/// The whole point of the host ordering edits: three people edit the same
/// things at once, and whichever guest's edits arrive first, everyone ends
/// on the same document. One run has a rename refused because the item was
/// deleted, and an insert moved because the list got shorter.
#[test]
fn a_shared_project_converges_whichever_order_edits_arrive_in() {
    for order in [[0, 1], [1, 0]] {
        let (doc, x, y) = two_solids();
        let host = end(&doc, false);
        let (a, b) = (end(&doc, true), end(&doc, true));

        a.store.commit(rename(x, "a first")).unwrap();
        let added = loose_item(&a.store);
        a.store.commit(rename(x, "a second")).unwrap();
        b.store.commit(Op::RemoveItem { id: x }).unwrap();
        b.store.commit(rename(y, "b")).unwrap();
        host.store.commit(rename(y, "host")).unwrap();

        deliver(&host, &[&a, &b], &order);

        let settled = host.store.snapshot();
        assert_eq!(*a.store.snapshot(), *settled, "guest a, order {order:?}");
        assert_eq!(*b.store.snapshot(), *settled, "guest b, order {order:?}");
        assert!(settled.item(added).is_some(), "the new item survives");
        assert_eq!(name_of(&host.store, y).as_deref(), Some("b"));
    }
}

/// Undo in a shared project is each person's own. A step someone else has
/// since written over is skipped rather than taking their change away, and
/// the step before it is undone instead.
#[test]
fn undo_leaves_what_someone_else_has_since_changed() {
    let (doc, x, y) = two_solids();
    let host = end(&doc, false);
    host.store.commit(rename(y, "mine")).unwrap();
    host.store.commit(rename(x, "mine")).unwrap();
    let tag = RemoteTag { peer: 1, id: 1 };
    let theirs = rename(x, "theirs");
    host.store.commit_remote(&theirs, None, tag).unwrap();

    host.store.undo().unwrap();

    assert_eq!(name_of(&host.store, x).as_deref(), Some("theirs"));
    assert_eq!(name_of(&host.store, y).as_deref(), Some("White solid"));
    assert!(!host.store.can_undo());
}

/// Edits made while the host was away come back onto the host's document.
/// The ones that touch nothing the host's side changed are applied and
/// sent. The one that does is held back with the host's version in place.
#[test]
fn edits_made_apart_are_merged_and_a_collision_is_held_back() {
    let (doc, x, y) = two_solids();
    let guest = end(&doc, true);
    guest.store.cast_adrift();
    guest.store.commit(rename(x, "mine")).unwrap();
    guest.store.commit(rename(y, "mine")).unwrap();
    let added = loose_item(&guest.store);
    assert!(guest.heard.lock().is_empty(), "nothing is sent while away");

    let mut theirs = doc.clone();
    apply(&mut theirs, &rename(x, "theirs")).unwrap();
    let (conflicts, refused, _) = guest.store.rejoin(theirs, 0);

    assert!(refused.is_empty());
    assert_eq!(conflicts.len(), 1);
    let held: Vec<&Op> = conflicts[0].ops.iter().map(|(op, _)| op).collect();
    assert_eq!(held, vec![&rename(x, "mine")]);
    assert_eq!(name_of(&guest.store, x).as_deref(), Some("theirs"));
    assert_eq!(name_of(&guest.store, y).as_deref(), Some("mine"));
    assert!(guest.store.snapshot().item(added).is_some());
    assert_eq!(guest.heard.lock().len(), 2, "the clean edits are sent");
}

/// A batch that stops landing part-way leaves the document as it was:
/// what landed of it is taken back, however batches sit inside it. A
/// guest lands every unanswered edit again whenever someone else's goes
/// in underneath, and one that no longer lands must leave nothing behind
/// for the next to land on.
#[test]
fn a_batch_that_stops_landing_part_way_leaves_nothing_behind() {
    let (doc, x, y) = two_solids();
    let inner = vec![rename(y, "of it"), rename(Uuid::now_v7(), "nothing")];
    let ops = vec![rename(x, "half"), Op::Batch { ops: inner }];
    let mut landed = doc.clone();
    assert!(land(&mut landed, &Op::Batch { ops }, None).is_err());
    assert_eq!(json(&landed), json(&doc));
}

/// Two people who file things into one folder both keep theirs, whatever
/// else either did to its list: the edit lands cut down to what its
/// author changed. Every solid and every imported file goes into a
/// folder with the others of its kind, so this is how most unanswered
/// edits land once someone else has made one of their own.
#[test]
fn a_folders_children_merge_by_what_each_person_changed() {
    let seed = DocumentStore::new(Document::new());
    let folder = make_folder(&seed, "Solids");
    let [a, b, c, m, t] = std::array::from_fn(|_| Uuid::now_v7());
    let set = |children: &[Uuid]| Op::SetFolderChildren {
        folder,
        children: children.to_vec(),
    };
    // What the list was where the edit was made, what the edit made it,
    // what it is here now, and what it has to come to.
    let cases: [[&[Uuid]; 4]; 3] = [
        // Both filed something.
        [&[a, b], &[a, b, m], &[a, b, t], &[a, b, m, t]],
        // One took something out, and the other filed something.
        [&[a, b, c], &[a, c], &[a, b, c, t], &[a, c, t]],
        // One put them in another order and filed something, and the
        // other took one out and filed another.
        [&[a, b, c], &[c, a, b, m], &[a, c, t], &[c, t, a, m]],
    ];
    for [was, mine, now, merged] in cases {
        let mut doc = Document::clone(&seed.snapshot());
        apply(&mut doc, &set(now)).unwrap();
        land(&mut doc, &set(mine), Some(&set(was))).unwrap();
        assert_eq!(doc.folder(folder).unwrap().children, merged);
    }
}

/// The host took one of this guest's edits before it was lost, and never
/// got to answer it. That edit is the guest's own doing: what the guest
/// went on to do to the same thing is not held back as if someone else
/// had been at it. When someone else has, it still is.
#[test]
fn an_edit_the_host_took_is_not_held_against_the_guest_that_made_it() {
    let (doc, x, y) = two_solids();
    for theirs_too in [false, true] {
        let guest = end(&doc, true);
        // Sent and not answered when the host is lost. Then more is
        // done to the same item, and to another.
        guest.store.commit(rename(x, "first")).unwrap();
        guest.store.cast_adrift();
        guest.store.commit(rename(x, "second")).unwrap();
        guest.store.commit(rename(y, "mine")).unwrap();

        // The host had taken the first, which was the guest's edit 1.
        let mut theirs = doc.clone();
        apply(&mut theirs, &rename(x, "first")).unwrap();
        if theirs_too {
            apply(&mut theirs, &rename(x, "theirs")).unwrap();
        }
        let (conflicts, refused, _) = guest.store.rejoin(theirs, 1);

        assert!(refused.is_empty());
        let name = name_of(&guest.store, x);
        if theirs_too {
            assert_eq!(conflicts.len(), 1, "someone else changed it since");
            assert_eq!(name.as_deref(), Some("theirs"));
        } else {
            assert!(conflicts.is_empty(), "nobody else touched it");
            assert_eq!(name.as_deref(), Some("second"));
        }
        assert_eq!(name_of(&guest.store, y).as_deref(), Some("mine"));
    }
}

/// An edit made on top of a held one is held with it. Deleting an item
/// the host's side renamed and then undoing the delete comes to nothing,
/// so the undo is not judged on the host's document, where the item was
/// never gone. Keeping mine puts my item back, and does not take it out.
#[test]
fn an_edit_made_on_top_of_a_held_one_is_held_with_it() {
    let (doc, x, _) = two_solids();
    let guest = end(&doc, true);
    guest.store.cast_adrift();
    guest.store.commit(Op::RemoveItem { id: x }).unwrap();
    guest.store.undo().unwrap();

    let mut theirs = doc.clone();
    apply(&mut theirs, &rename(x, "theirs")).unwrap();
    let (conflicts, refused, _) = guest.store.rejoin(theirs, 0);

    assert_eq!((conflicts.len(), refused.len()), (1, 0));
    assert_eq!(conflicts[0].ops.len(), 2, "the delete and its undo");
    assert_eq!(name_of(&guest.store, x).as_deref(), Some("theirs"));
    for (op, was) in &conflicts[0].ops {
        guest.store.commit_over(op.clone(), was).unwrap();
    }
    assert_eq!(name_of(&guest.store, x).as_deref(), Some("White solid"));
}

/// An op writes a layer's whole effect stack, and here two people each
/// change a different effect on one layer. Both changes stand: at the
/// same moment, through an undo, and when one was made while the host was
/// away. Only the same parameter changed by both is a collision.
#[test]
fn two_people_can_change_different_effects_on_one_layer() {
    use crate::anim::Property;
    let (seed, comp, layer) = doc_with_layer();
    let blur = || crate::fx::builtins::instantiate("blur").expect("the catalogue knows it");
    let stack = vec![blur(), blur()];
    let (first, second) = (stack[0].id, stack[1].id);
    let write = |store: &DocumentStore, effects: Vec<EffectInstance>| {
        let op = Op::SetLayerEffects {
            comp,
            layer,
            effects,
        };
        store.commit(op).unwrap();
    };
    write(&seed, stack.clone());
    let doc = Document::clone(&seed.snapshot());

    let read = |store: &DocumentStore| -> Vec<EffectInstance> {
        let doc = store.snapshot();
        let layers = &doc.comp(comp).unwrap().layers;
        layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap()
            .effects
            .clone()
    };
    let with = |mut effects: Vec<EffectInstance>, effect: Uuid, radius: f64| {
        let on = effects.iter_mut().find(|e| e.id == effect).unwrap();
        let param = on.params.iter_mut().find(|p| p.id == "radius").unwrap();
        param.value = EffectValue::Float(Property::fixed(radius));
        effects
    };
    let set = |store: &DocumentStore, effect: Uuid, radius: f64| {
        write(store, with(read(store), effect, radius));
    };

    let (host, guest) = (end(&doc, false), end(&doc, true));
    set(&host.store, first, 3.0);
    set(&guest.store, second, 7.0);
    deliver(&host, &[&guest], &[0]);
    let both = with(with(stack.clone(), first, 3.0), second, 7.0);
    assert_eq!(read(&host.store), both);
    assert_eq!(read(&guest.store), both);

    guest.store.undo().unwrap();
    deliver(&host, &[&guest], &[0]);
    let hosts = with(stack.clone(), first, 3.0);
    assert_eq!(read(&host.store), hosts, "undo takes back the guest's own");
    assert_eq!(read(&guest.store), hosts);

    let away = end(&doc, true);
    away.store.cast_adrift();
    set(&away.store, second, 7.0);
    set(&away.store, first, 5.0);
    let theirs = Document::clone(&host.store.snapshot());
    let (conflicts, refused, _) = away.store.rejoin(theirs, 0);
    assert_eq!((conflicts.len(), refused.len()), (1, 0));
    assert_eq!(
        read(&away.store),
        both,
        "the host's stands where both changed"
    );

    let (op, was) = conflicts[0].ops[0].clone();
    away.store.commit_over(op, &was).unwrap();
    let mine = with(with(stack, first, 5.0), second, 7.0);
    assert_eq!(read(&away.store), mine, "keeping mine leaves the rest");
}
