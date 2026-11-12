//! Tests for an export the Viewer's settings never reach.

use super::*;

/// **The export cannot see the Viewer's view**, and it is neutral by
/// *construction* rather than by discipline: `export::run` builds its own
/// `HeadlessRenderer` and nothing ever calls the setter on it, then renders
/// each frame through `render_preview` exactly as this does.
///
/// So the property under test is that the view is renderer-owned state. The
/// obvious regression — making it a global, a static or a thread-local, all
/// of which would look fine in every preview test — fails here, because the
/// export's fresh renderer would then inherit a view somebody else set. The
/// first assertion also proves the view is doing something at all, so this
/// cannot pass by the display transform being a no-op.
#[test]
fn an_export_renders_neutral_whatever_the_viewer_is_set_to() {
    let mut viewer = match HeadlessRenderer::shared() {
        Ok(r) => r,
        Err(_) => {
            lumit_gpu::no_adapter();
            return;
        }
    };
    let (store, comp_id) = doc_with_solid(LinearColour([0.18, 0.18, 0.18, 1.0]), 8, 8);
    let doc = store.snapshot();
    let q = crate::plan::Quality::default();

    let (neutral, _, _) = viewer
        .render_preview(&doc, comp_id, 0, q, 1.0)
        .expect("neutral render");

    viewer.set_display_view(lumit_gpu::DisplayParams::from_stops(2.0, true));
    let (exposed, _, _) = viewer
        .render_preview(&doc, comp_id, 0, q, 1.0)
        .expect("exposed render");
    assert_ne!(
        exposed, neutral,
        "two stops and a tone map must visibly change the preview, or this \
         test would pass on a display transform that does nothing"
    );

    // What the exporter does, in the order it does it (`export::run`): its
    // own renderer, then `render_preview` per frame at full quality.
    let mut exporter = HeadlessRenderer::new().expect("export renderer");
    assert!(
        exporter.display_view().is_neutral(),
        "a fresh renderer starts neutral, which is what makes export neutral"
    );
    let (exported, _, _) = exporter
        .render_preview(&doc, comp_id, 0, q, 1.0)
        .expect("export render");
    assert_eq!(
        exported, neutral,
        "the export's bytes are the neutral bytes, with the Viewer set to \
         two stops and a tone map"
    );
}
