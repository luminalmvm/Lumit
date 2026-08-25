//! Decoding a real media file, named by the environment.
//!
//! A harness rather than a fixture: the repo carries no video, and the crashes
//! worth chasing are the ones a particular file provokes. Point
//! `LUMIT_TEST_MEDIA` at a clip and this probes it, opens it, and pulls frames
//! across the whole span the way a comp render does — which is the difference
//! between "the file imported" (a probe) and "the comp rendered" (this).
//!
//! Ignored by default so CI does not depend on a path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

#[test]
#[ignore = "harness: set LUMIT_TEST_MEDIA to a clip"]
fn decode_every_frame_of_a_real_clip() {
    let Ok(path) = std::env::var("LUMIT_TEST_MEDIA") else {
        eprintln!("set LUMIT_TEST_MEDIA to the clip to decode");
        return;
    };
    let path = Path::new(&path);
    let probe = lumit_media::probe::probe(path).expect("probe the clip");
    let video = probe.video.as_ref().expect("the clip has a video stream");
    eprintln!(
        "probe: {}x{} @ {} fps, {} s",
        video.width,
        video.height,
        video.fps(),
        probe.duration_seconds,
    );

    let index = lumit_media::index::build_frame_index(path).expect("build the frame index");
    let mut dec = lumit_media::decode::VideoDecoder::open(path, index).expect("open the decoder");
    let count = dec.frame_count();
    eprintln!("decoder reports {count} frames");
    assert!(
        count > 0,
        "a clip that probes must decode at least one frame"
    );

    // Every frame, in order, at native width — the comp render's pattern.
    for f in 0..count {
        let frame = dec
            .frame_rgba(f, None)
            .unwrap_or_else(|e| panic!("frame {f} of {count} failed: {e}"));
        assert_eq!(
            frame.rgba.len(),
            frame.width as usize * frame.height as usize * 4,
            "frame {f} came back the wrong size"
        );
    }
    eprintln!("decoded all {count} frames");
}
