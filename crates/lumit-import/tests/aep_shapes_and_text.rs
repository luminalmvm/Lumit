//! A project saved by After Effects 2026 with shape and text layers in it,
//! read straight from the `.aep` and mapped.
//!
//! The rectangle is AskNK's test from his review: one shape crossing the frame
//! as it fades in, which arrived drawing nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use lumit_core::model::{Document, Layer, LayerKind, ProjectItem};

fn imported() -> Document {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shapes-and-text.aep");
    let bundle = lumit_import::open_aep(&file).expect("the project opens");
    lumit_import::map_capture(&bundle.capture).0
}

fn layer<'a>(doc: &'a Document, name: &str) -> &'a Layer {
    doc.items
        .iter()
        .find_map(|item| match item {
            ProjectItem::Composition(comp) => comp.layers.iter().find(|l| l.name == name),
            _ => None,
        })
        .expect("the layer is there")
}

#[test]
fn shape_contents_arrive_where_after_effects_draws_them() {
    let doc = imported();

    // A 400 × 200 rectangle with rounded corners, filled and outlined.
    let LayerKind::Shape { contents } = &layer(&doc, "Rect").kind else {
        panic!("a shape layer");
    };
    assert_eq!(contents.len(), 2, "one fill and one stroke");
    // Measured round the outside of the 8 px outline, so 4 px more each side.
    let bounds = lumit_core::shape::contents_bounds(contents, 0.0).unwrap();
    assert_eq!(bounds, (-204.0, -104.0, 204.0, 104.0));
    assert!(contents.iter().any(|item| item.fill.is_some()));
    assert!(contents.iter().any(|item| item.stroke_width == 8.0));
    assert_eq!(contents[0].path.vertices.len(), 8, "rounded corners");

    // The drawn path is in pixels, where a mask's points are fractions of the
    // layer, and its trim is keyed.
    let LayerKind::Shape { contents } = &layer(&doc, "Mix").kind else {
        panic!("a shape layer");
    };
    let path = contents
        .iter()
        .find(|item| item.name == "Path 1")
        .expect("the drawn path");
    let xs: Vec<f64> = path.path.vertices.iter().map(|v| v.pos.0).collect();
    assert_eq!(xs, vec![-200.0, 0.0, 200.0]);
    assert_eq!(path.trim_end.value_at(2.0), 100.0);
    assert_eq!(path.trim_end.value_at(0.0), 0.0);
}

#[test]
fn text_layers_keep_their_words_and_how_they_are_set() {
    use lumit_core::text::TextAlign;

    let doc = imported();

    let LayerKind::Text { document } = &layer(&doc, "Title").kind else {
        panic!("a text layer");
    };
    assert_eq!(document.text, "Hello Lumit");
    assert_eq!(document.size, 100.0);
    assert_eq!(document.paragraph.align, TextAlign::Centre);
    assert_eq!(document.paragraph.box_width, 0.0);

    let LayerKind::Text { document } = &layer(&doc, "Caption").kind else {
        panic!("a text layer");
    };
    assert_eq!(document.text, "A second line of words in a box");
    assert_eq!(document.size, 48.0);
    assert_eq!(document.paragraph.box_width, 600.0);
    assert_eq!(document.style.tracking, 50.0);
    assert!(document.style.stroke_on);
    assert_eq!(document.style.stroke_width, 4.0);
}
