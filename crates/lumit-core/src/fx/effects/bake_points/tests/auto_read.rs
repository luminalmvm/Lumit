//! Tests for when Auto reads the bake.

use super::*;

/// Auto hands on the bake only while what is above the effect is what it
/// was baked from. Moving the layer, or changing what is below the
/// effect, keeps it. Changing an effect above or a driver feeding one
/// does not. Read hands on the bake whatever, and nothing past its end.
#[test]
fn auto_reads_a_bake_only_while_what_is_above_it_is_unchanged() {
    let (plain, comp_id) = board();
    let edited = |f: &dyn Fn(&mut Layer)| {
        let mut doc = plain.clone();
        f(&mut doc.comp_mut(comp_id).unwrap().layers[0]);
        Arc::new(doc)
    };
    let base = edited(&|_| {});
    let at = |doc: &Arc<Document>, t: f64| {
        let comp = doc.comp(comp_id).unwrap();
        let layer = &comp.layers[0];
        reading(doc, comp, layer, &layer.effects[1], t)
    };
    let frame = |doc: &Arc<Document>, t: f64| match at(doc, t) {
        Reading::Baked(_, unique) => Some(unique),
        _ => None,
    };
    let live = |doc: &Arc<Document>, t: f64| matches!(at(doc, t), Reading::Live);
    let comp = base.comp(comp_id).unwrap();
    let layer = &comp.layers[0];
    let effect = layer.effects[1].id;
    assert_eq!(fingerprint(&base, comp, layer, Uuid::now_v7()), None);
    assert!(live(&base, 0.0), "nothing is baked yet");

    let print = fingerprint(&base, comp, layer, effect).unwrap();
    publish(effect, bake(print));
    assert_eq!(frame(&base, 2.0 / 24.0), Some(1));
    assert_eq!(frame(&base, 4.0 / 24.0), Some(3));
    let read = at(&base, 2.0 / 24.0).stream(Projection::FLAT).unwrap();
    assert!(read == frames()[2], "Auto hands on the baked frame");

    let moved = edited(&|l| {
        l.transform.position_x = Property::fixed(400.0);
        l.name = "Moved".into();
    });
    assert_eq!(frame(&moved, 2.0 / 24.0), Some(1), "moving keeps a bake");
    let below = edited(&|l| {
        set(&mut l.effects[2], "max_distance", 3.0);
        set(&mut l.graph.nodes[1], "a", 9.0);
    });
    assert_eq!(frame(&below, 2.0 / 24.0), Some(1), "so does what is below");

    let above = edited(&|l| set(&mut l.effects[0], "to", 20.0));
    assert!(live(&above, 2.0 / 24.0), "an effect above counts");
    let driven = edited(&|l| set(&mut l.graph.nodes[0], "a", 9.0));
    assert!(live(&driven, 2.0 / 24.0), "and a driver feeding one");
    assert!(live(&base, 9.0), "past the bake is live");

    let read = edited(&|l| {
        set(&mut l.effects[0], "to", 20.0);
        mode(&mut l.effects[1], MODE_READ);
    });
    assert_eq!(frame(&read, 2.0 / 24.0), Some(1), "Read takes a stale bake");
    let past = at(&read, 9.0).stream(Projection::FLAT).unwrap();
    assert!(past.is_empty(), "and nothing past its end");
    let bypass = edited(&|l| mode(&mut l.effects[1], MODE_BYPASS));
    assert!(live(&bypass, 2.0 / 24.0));

    // What the effect resolves to says which frame of which bake is read,
    // so a picture drawn from one is never served for another.
    let derived = |doc: &Arc<Document>, t: f64| {
        let layer = &doc.comp(comp_id).unwrap().layers[0];
        let context = ExpressionContext {
            document: Arc::clone(doc),
            comp: Some(comp_id),
            layer: Some(layer.id),
            comp_time: t,
            current_depth: 0,
            inputs: None,
        };
        let stack = crate::fx::resolved::resolve_stack(
            &layer.effects,
            t,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(context),
        );
        let params = stack.get(1).expect("the bake resolves").params;
        (
            params.choice(BakePoints::DERIVED_BAKE, u32::MAX),
            params.int(BakePoints::DERIVED_FRAME, i32::MAX),
        )
    };
    let held = baked(effect).unwrap();
    let [a, b, c, d, ..] = held.content;
    let head = u32::from_le_bytes([a, b, c, d]);
    assert_eq!(derived(&base, 2.0 / 24.0), (head, 1));
    assert_eq!(derived(&bypass, 2.0 / 24.0), (0, -1));
    assert_eq!(derived(&read, 9.0), (0, -2));

    retain(|id| *id != effect);
    assert!(baked(effect).is_none());
    assert!(matches!(at(&read, 2.0 / 24.0), Reading::Nothing));
}
