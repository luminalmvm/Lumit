//! What the helper is for: a file that breaks the reader ends the helper and
//! not the program that asked.
//!
//! This lives here and not in `lumit-media` for one Cargo reason:
//! `CARGO_BIN_EXE_lumit-media-broker` exists only inside the package that
//! owns the binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use lumit_media::ai::fixture::{document, Layer};
use lumit_media::ai::{open, read_layer, read_layer_within, BROKER_EXE_ENV};
use lumit_media::MediaError;

#[test]
fn a_file_is_read_through_the_helper_and_a_helper_that_fails_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("art.ai");
    let layers = [
        Layer::solid("Background", [0, 0, 6, 8], [255, 0, 0]),
        Layer::solid("Hat", [1, 2, 3, 6], [0, 0, 255]),
    ];
    std::fs::write(&good, document(8, 6, &layers)).unwrap();

    // The one test in this program, so nothing else reads the variable.
    std::env::set_var(BROKER_EXE_ENV, env!("CARGO_BIN_EXE_lumit-media-broker"));
    let doc = open(&good).unwrap();
    assert_eq!((doc.width, doc.height, doc.layers.len()), (8, 6, 2));
    assert_eq!(doc.layers[1].name, "Hat");
    let frame = read_layer(&good, Some(1), None).unwrap();
    assert_eq!((frame.width, frame.height), (8, 6));
    let px = |x: usize, y: usize| &frame.rgba[(y * 8 + x) * 4..][..4];
    assert_eq!(px(2, 1), [0, 0, 255, 255]);
    assert_eq!(px(0, 0), [0; 4], "the background is another layer");

    // The helper's own reason for refusing a file comes back with it.
    let reason = |result: Result<_, MediaError>| match result {
        Err(MediaError::Ai(reason)) => reason,
        _ => panic!("the read should have failed"),
    };
    assert_eq!(
        reason(read_layer(&good, Some(9), None).map(|_| ())),
        "no such layer"
    );
    let cut_short = dir.path().join("cut.ai");
    std::fs::write(&cut_short, &document(8, 6, &layers)[..200]).unwrap();
    reason(open(&cut_short).map(|_| ()));
    reason(read_layer(&cut_short, None, None).map(|_| ()));

    // A helper that runs out of time is stopped, and that is an error too.
    // The artboard is a large one, so no helper has drawn it before the
    // caller has looked.
    let large = dir.path().join("large.ai");
    std::fs::write(&large, document(8192, 8192, &layers)).unwrap();
    assert_eq!(
        reason(read_layer_within(&large, None, None, Duration::ZERO).map(|_| ())),
        "the file took too long to read"
    );

    // A program that is not the helper answers with something else. This
    // test program stands in for one: it prints its own report. Nothing is
    // set aside for the frame those bytes would describe.
    std::env::set_var(BROKER_EXE_ENV, std::env::current_exe().unwrap());
    reason(open(&good).map(|_| ()));
    reason(read_layer(&good, None, None).map(|_| ()));
}
