//! Tests for edits that keep every frame key.

use super::*;

/// Edits that cannot reach a pixel keep every key — the whole reason the
/// cache is content-keyed rather than emptied on every commit. The work
/// area is where the playhead loops, a marker is a label, a label colour is
/// Timeline chrome, and volume is sound: none of them is in the picture.
#[test]
fn picture_free_edits_keep_every_key() {
    let doc = Document::new();
    let mut comp = comp_with(vec![text_layer("hi", 0.0, 8.0, 0.0)]);
    let before = key(&doc, &comp, 1.0);

    comp.work_area = Some((secs(1.0), secs(2.0)));
    assert_eq!(
        before,
        key(&doc, &comp, 1.0),
        "the work area is not content"
    );

    comp.markers.push(lumit_core::markers::Marker {
        id: Uuid::now_v7(),
        time: secs(1.0),
        duration: None,
        label: "beat".into(),
        kind: lumit_core::markers::MarkerKind::Beat { confidence: 1.0 },
        colour: None,
        extra: serde_json::Map::new(),
    });
    assert_eq!(before, key(&doc, &comp, 1.0), "a marker is a label");

    comp.layers[0].label = 5;
    comp.layers[0].volume_db = Property::fixed(-6.0);
    comp.layers[0].switches.audible = false;
    comp.layers[0].switches.shy = true;
    comp.layers[0].switches.locked = true;
    assert_eq!(
        before,
        key(&doc, &comp, 1.0),
        "label, volume, audio and the Timeline switches change no pixel"
    );

    // And a node graph's canvas is presentation too (docs/impl/
    // node-graph-comp.md §2.5): moving a box, twirling one open and
    // naming a wash draw nothing at all.
    let (gdoc, graphed, read, ..) = node_graph_doc();
    let base = key(&gdoc, &graphed, 1.0);
    let mut arranged = graphed.clone();
    if let Some(g) = arranged.graph.as_mut() {
        g.layout.push((read, [640.0, 480.0]));
        g.exposed.push(read);
        g.groups.push(lumit_core::comp_graph::GraphGroup {
            name: "the wash".into(),
            colour: 3,
            members: vec![read],
        });
    }
    assert_eq!(
        base,
        key(&gdoc, &arranged, 1.0),
        "layout, exposure and groups are the canvas, not the picture"
    );
}
