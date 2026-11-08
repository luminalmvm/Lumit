//! A sound for Audio spectrum's picture, since the plate is silent.

use lumit_core::model::{Layer, LayerKind};
use lumit_core::time::Rational;
use uuid::Uuid;

/// The rate the chord is written at, and how long it lasts in seconds.
const RATE: u32 = 48_000;
const SECONDS: u32 = 2;

/// Two seconds of a rich chord as a 16-bit stereo WAV in the temp folder, with
/// enough partials falling away up the range to light every band.
pub(super) fn written() -> std::path::PathBuf {
    let path = std::env::temp_dir().join("lumit-fx-chord.wav");
    let frames = RATE * SECONDS;
    let mut bytes = Vec::with_capacity(44 + frames as usize * 4);
    let data = frames * 4;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for i in 0..frames {
        let t = f64::from(i) / f64::from(RATE);
        let mut v = 0.0;
        for k in 1..=48 {
            let hz = 55.0 * 1.13f64.powi(k);
            v += (std::f64::consts::TAU * hz * t).sin() * 0.9 / f64::from(k).sqrt() / 8.0;
        }
        let sample = (v.clamp(-1.0, 1.0) * 32000.0) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&path, bytes).expect("the chord is written");
    path
}

/// A layer of nothing but that sound.
pub(super) fn layer(item: Uuid, span: Rational) -> Layer {
    let mut l = super::layer("Chord", LayerKind::Footage { item }, span);
    l.audio_only = true;
    l
}
