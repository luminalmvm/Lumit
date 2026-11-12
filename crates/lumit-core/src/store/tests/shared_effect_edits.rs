//! Tests for two people changing effects on one layer.

use super::*;

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
