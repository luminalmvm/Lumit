//! Preview-engine tests for `AppState` (moved verbatim from app_state.rs).

use super::preview::PreviewEngine;
use lumit_media::index::tests_support::fixture;
use std::time::Duration;

/// End-to-end: request a frame the way the Viewer does; receive pixels.
#[test]
fn preview_engine_decodes_requested_frame_at_requested_size() {
    let dir = tempfile::tempdir().unwrap();
    let Some(file) = fixture(dir.path()) else {
        eprintln!("skipping: no ffmpeg CLI available");
        return;
    };
    let engine = PreviewEngine::default();
    let id = uuid::Uuid::now_v7();
    engine.request(id, file, 45, Some(160));
    let result = engine
        .results
        .recv_timeout(Duration::from_secs(20))
        .expect("engine replied")
        .expect("decode succeeded");
    let super::preview::PreviewResult::Footage(px) = result else {
        panic!("expected a footage frame");
    };
    assert_eq!(px.item, id);
    assert_eq!(px.frame, 45);
    assert_eq!((px.width, px.height), (160, 120));
    assert_eq!(px.rgba.len(), 160 * 120 * 4);
}

/// Latest-wins: flood requests; the engine may skip stale ones and the
/// final delivered frame is the newest request.
#[test]
fn preview_engine_latest_request_wins() {
    let dir = tempfile::tempdir().unwrap();
    let Some(file) = fixture(dir.path()) else {
        eprintln!("skipping: no ffmpeg CLI available");
        return;
    };
    let engine = PreviewEngine::default();
    let id = uuid::Uuid::now_v7();
    for n in 0..60 {
        engine.request(id, file.clone(), n, None);
    }
    let mut last = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match engine.results.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(super::preview::PreviewResult::Footage(px))) => {
                last = Some(px.frame);
                if px.frame == 59 {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => panic!("decode failed: {e}"),
            Err(_) => {
                if last == Some(59) {
                    break;
                }
            }
        }
    }
    assert_eq!(last, Some(59), "newest request must be the one served last");
}
