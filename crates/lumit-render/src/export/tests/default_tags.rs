//! Tests for what the default colour space writes and says.

use super::*;

/// The default space writes the picture the Viewer shows, tagged Rec. 709
/// (`the_colour_space_reaches_the_containers_colr_box` reads the tag). A file
/// tagged with the sRGB curve is converted on upload and comes back darker
/// than the Viewer.
#[test]
fn the_default_export_is_the_viewers_picture_tagged_rec_709() {
    let (doc, comp) = solid_doc(32, 16);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("default.mp4");
    let mut sp = spec(ExportFormat::Video(VideoCodec::H264), 32, 16);
    sp.range = Some((0, 2));
    assert_eq!(sp.colour_space, ColourSpace::default());
    let Some(result) = run_now(&doc, comp, &path, &sp) else {
        return;
    };
    result.expect("export runs");

    // Read back, the file is the Viewer's picture, give or take the rounding
    // of eight-bit YCbCr. A wrong matrix or range is off by ten or more.
    let index = lumit_media::index::build_frame_index(&path).unwrap();
    let mut decoder = lumit_media::decode::VideoDecoder::open_with(&path, index, false).unwrap();
    let file = decoder.frame_rgba(0, None).unwrap();
    let mut renderer = crate::headless::HeadlessRenderer::shared().unwrap();
    let (viewer, _, _) = renderer
        .render_preview(&doc, comp, 0, crate::plan::Quality::default(), 1.0)
        .unwrap();
    let centre = (8 * 32 + 16) * 4;
    for c in 0..3 {
        let (got, want) = (file.rgba[centre + c], viewer[centre + c]);
        assert!(
            got.abs_diff(want) <= 3,
            "channel {c}: the file has {got}, the Viewer {want}"
        );
    }
}
