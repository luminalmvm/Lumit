//! Tests for the Retime property through undo.

use super::*;

/// The Retime *property* round-trips through undo, and it is what
/// `source_time_at` answers with — the mapping the render plan and the
/// cache key both read.
#[test]
fn retime_property_round_trips_and_maps_source_time() {
    use crate::model::Layer;
    use crate::time::Rational;
    let store = DocumentStore::new(Document::new());
    let (ops, comp_id) = scripted_ops(&store.snapshot());
    let mut layer_id = None;
    for op in &ops {
        if let Op::AddLayer { layer, .. } = op {
            layer_id = Some(layer.id);
        }
    }
    for op in ops {
        store.commit(op).unwrap();
    }
    let layer_id = layer_id.unwrap();
    let layer_of = |doc: &Document| {
        doc.comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap()
            .clone()
    };

    // No retime: the layer reads its source at its own clock.
    assert!((layer_of(&store.snapshot()).source_time_at(4.0) - 4.0).abs() < 1e-9);

    // Identity over ten seconds, then half of it: local 4 → source 2.
    let ten = Rational::new(10, 1).unwrap();
    let mut retime = Layer::identity_retime(Rational::ZERO, ten);
    if let crate::anim::Animation::Keyframed(keys) = &mut retime.animation {
        keys[1].value = 5.0;
    }
    store
        .commit(Op::SetRetimeProperty {
            comp: comp_id,
            layer: layer_id,
            retime: Some(retime),
        })
        .unwrap();
    assert!((layer_of(&store.snapshot()).source_time_at(4.0) - 2.0).abs() < 1e-9);

    store.undo().unwrap();
    let layer = layer_of(&store.snapshot());
    assert!(layer.retime.is_none());
    assert!((layer.source_time_at(4.0) - 4.0).abs() < 1e-9);
}
