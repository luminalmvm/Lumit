//! Where the page hands footage to the engine (flutter_ui/web/media.js). A
//! file is opened and read there before the engine hears of it, under a path
//! of its own, `/web/<id>/<name>`. Everything the engine then asks about that
//! path is answered from what was handed in here (`lumit_media::web`).

use wasm_bindgen::prelude::wasm_bindgen;

/// What the page found in a file it opened. `probe` is a
/// `lumit_media::probe::MediaProbe` as JSON, `ends` the file's first and
/// last 64 KiB, and `pts` and `keys` each frame's moment in `ticks` a second
/// and whether it is a keyframe. False when `probe` doesn't read.
#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub fn lumit_web_register(
    path: String,
    probe: String,
    size: f64,
    modified: f64,
    ends: Vec<u8>,
    ticks: u32,
    pts: Vec<f64>,
    keys: Vec<u8>,
) -> bool {
    let Ok(probe) = serde_json::from_str(&probe) else {
        return false;
    };
    let frames = pts
        .iter()
        .zip(keys.iter().chain(std::iter::repeat(&0)))
        .map(|(&pts, &key)| (pts as i64, key != 0))
        .collect();
    lumit_media::web::register(
        path.into(),
        lumit_media::web::Opened {
            probe,
            size: size as u64,
            modified: modified as i64,
            ends,
            ticks,
            frames,
        },
    );
    true
}

/// A file's sound, interleaved stereo at `rate`.
#[wasm_bindgen]
pub fn lumit_web_sound(path: String, rate: u32, samples: Vec<f32>) {
    lumit_media::web::register_sound(std::path::Path::new(&path), rate, samples);
}

/// A frame the page decoded, as RGBA. The worker takes its next turn now, so
/// a picture drawn without this frame is drawn again.
#[wasm_bindgen]
pub fn lumit_web_frame(path: String, frame: usize, width: u32, height: u32, rgba: Vec<u8>) {
    lumit_media::web::frames::land(std::path::Path::new(&path), frame, width, height, rgba);
    crate::turns::wake();
}
