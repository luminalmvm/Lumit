//! Tests for the in use badge.

use super::*;

/// The `in use` badge: footage, a solid and a nested composition all count
/// as used the moment a layer names them, and an item nothing names does
/// not — whatever else the project holds.
#[test]
fn an_item_is_used_when_some_comps_layer_names_it() {
    let footage = bare_footage("a.mp4", "a.mp4", "");
    let used_footage = footage.id;
    let idle_footage = bare_footage("b.mp4", "b.mp4", "");
    let idle_id = idle_footage.id;

    let solid = SolidDef {
        id: Uuid::now_v7(),
        name: "White".into(),
        colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
        width: 64,
        height: 64,
        extra: serde_json::Map::new(),
    };
    let solid_id = solid.id;

    let mut inner = bare_comp("Inner");
    let inner_id = inner.id;
    inner
        .layers
        .push(bare_layer(LayerKind::Footage { item: used_footage }));

    let mut outer = bare_comp("Outer");
    outer
        .layers
        .push(bare_layer(LayerKind::Precomp { comp: inner_id }));
    outer
        .layers
        .push(bare_layer(LayerKind::Solid { def: solid_id }));
    // A drawn layer names no project item at all.
    outer.layers.push(bare_layer(LayerKind::Adjustment));

    let mut doc = Document::new();
    doc.items.push(ProjectItem::Footage(footage));
    doc.items.push(ProjectItem::Footage(idle_footage));
    doc.items.push(ProjectItem::Solid(solid));
    doc.items.push(ProjectItem::Composition(inner));
    doc.items.push(ProjectItem::Composition(outer));

    assert!(doc.item_is_used(used_footage));
    assert!(doc.item_is_used(solid_id));
    assert!(
        doc.item_is_used(inner_id),
        "a comp placed as a layer is used"
    );
    assert!(!doc.item_is_used(idle_id), "nothing places this footage");
    assert!(
        !doc.item_is_used(doc.items[4].id()),
        "the outer comp is in nothing, however much is in it"
    );
    assert!(!doc.item_is_used(Uuid::now_v7()), "an id nobody knows");
}
