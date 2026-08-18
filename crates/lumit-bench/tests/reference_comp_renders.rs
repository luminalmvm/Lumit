//! The reference comp builds, and the engine can make a frame of it.
//!
//! # In plain terms
//!
//! Before any budget can be measured, the thing being measured has to work.
//! This is the guard that catches the boring failures — a layer dropped from
//! the comp, an effect renamed out of the catalogue, media that will not
//! generate, a frame the cache cannot name — long before a timing run reports
//! them as a mysterious regression.
//!
//! It renders exactly one frame, and it is not `#[ignore]`d: the ordinary suite
//! runs it. The cost is bounded by reusing the generated media between runs —
//! see [`lumit_bench::media`] — so only the first run on a machine pays for the
//! two encodes.
//!
//! Skips politely on a machine with no ffmpeg, and on one with no GPU adapter
//! (the `no_adapter` convention the other GPU oracles use — set
//! `LUMIT_REQUIRE_GPU` on a machine that is supposed to have one and the skip
//! becomes a failure).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_bench::comp::LAYER_COUNT;
use lumit_bench::reference_comp;
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;

/// A stable directory, so the media is generated once per machine rather than
/// once per run. `tempfile` would be tidier and would re-encode 2400 frames of
/// 1080p60 every time the suite runs, which is the wrong trade for a test in
/// the ordinary suite.
fn media_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("lumit-bench-media")
}

#[test]
fn the_reference_comp_builds_and_renders_a_named_frame() {
    let (doc, comp_id) = match reference_comp(&media_dir()) {
        Ok(built) => built,
        Err(e) => {
            eprintln!("skipping: reference media unavailable ({e})");
            return;
        }
    };

    // docs/13 §1's five picture layers plus the audio layer.
    let comp = doc
        .comp(comp_id)
        .expect("the built comp is in the document");
    assert_eq!(
        comp.layers.len(),
        LAYER_COUNT,
        "docs/13 §1 layers: {:?}",
        comp.layers.iter().map(|l| &l.name).collect::<Vec<_>>()
    );
    assert_eq!((comp.width, comp.height), (1920, 1080));
    assert_eq!(comp.frame_rate.fps(), 60.0);

    let mut r = match HeadlessRenderer::new() {
        Ok(r) => r,
        Err(_) => {
            lumit_gpu::no_adapter();
            return;
        }
    };
    let doc = std::sync::Arc::new(doc);

    // Nameable: every source probed, so the frame can be filed in the content
    // cache. A scenario that cannot name its frames measures a cache that never
    // hits, which would quietly make every warm-playback number a cold one.
    assert!(
        r.frame_key(&doc, comp_id, 0, Quality::default()).is_some(),
        "frame 0 has no content name — a source failed to probe"
    );

    let (rgba, w, h) = r
        .render_rgba(&doc, comp_id, 0, 1.0)
        .expect("render frame 0 of the reference comp");
    assert_eq!((w, h), (1920, 1080));
    assert_eq!(rgba.len(), (w as usize) * (h as usize) * 4);

    // Not a flat frame. A comp whose layers had all silently dropped out would
    // still pass every assertion above and would still be *timed* — a
    // benchmark measuring an empty composite is the one failure this file
    // exists to make impossible.
    let first = &rgba[..4];
    assert!(
        rgba.chunks_exact(4).any(|px| px != first),
        "the reference comp composited to a single flat colour"
    );
}
