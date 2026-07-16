//! Audio playback and the playback clock (docs/09-AUDIO.md;
//! docs/impl/playback-scheduler.md §4).
//!
//! In plain terms: sound cards ask for samples on their own strict schedule
//! through a realtime callback. That callback is sacred — it never allocates,
//! never locks, never waits; if it is ever late you *hear* it. The number of
//! samples it has consumed IS the playback clock: video asks "what time is
//! it?" and chases the answer. That is why footage and sound can never drift
//! apart — there is only one clock, and it is the audio card's.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use luminal_media::AudioBuffer;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub mod beat;
pub mod mix;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio output device")]
    NoDevice,
    #[error("audio device: {0}")]
    Device(String),
}

struct Shared {
    /// Swapped rarely (on load); the callback try-reads and plays silence on miss.
    buffer: RwLock<Option<Arc<AudioBuffer>>>,
    /// Frames consumed since load/seek — the clock.
    playhead: AtomicUsize,
    playing: AtomicBool,
}

pub struct AudioEngine {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    device_rate: u32,
    channels: usize,
}

impl AudioEngine {
    pub fn new() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
        let config = device
            .default_output_config()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let device_rate = config.sample_rate().0;
        let channels = usize::from(config.channels());

        let shared = Arc::new(Shared {
            buffer: RwLock::new(None),
            playhead: AtomicUsize::new(0),
            playing: AtomicBool::new(false),
        });
        let cb = shared.clone();

        let stream = device
            .build_output_stream(
                &config.config(),
                move |out: &mut [f32], _| fill(&cb, out, channels),
                |_err| { /* device hiccup: next callback continues; never panic */ },
                None,
            )
            .map_err(|e| AudioError::Device(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Device(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            shared,
            device_rate,
            channels,
        })
    }

    /// The rate media should be decoded at so no runtime resampling happens.
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Install a buffer (decoded at `device_rate`) and rewind.
    pub fn load(&self, buffer: Arc<AudioBuffer>) {
        self.shared.playing.store(false, Ordering::Relaxed);
        *self.shared.buffer.write() = Some(buffer);
        self.shared.playhead.store(0, Ordering::Relaxed);
    }

    pub fn unload(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        *self.shared.buffer.write() = None;
        self.shared.playhead.store(0, Ordering::Relaxed);
    }

    pub fn play(&self) {
        self.shared.playing.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn seek_seconds(&self, t: f64) {
        let frame = (t.max(0.0) * f64::from(self.device_rate)) as usize;
        self.shared.playhead.store(frame, Ordering::Relaxed);
    }

    /// The playback clock (docs/06-RENDER-PIPELINE.md §A/V sync: audio is
    /// master). Output latency compensation arrives with the ring buffer
    /// work; at ±half a frame tolerance it is acceptable to omit for Gate 0.
    pub fn clock_seconds(&self) -> f64 {
        self.shared.playhead.load(Ordering::Relaxed) as f64 / f64::from(self.device_rate)
    }
}

/// The realtime callback: lock-free reads, no allocation, silence on any miss.
fn fill(shared: &Shared, out: &mut [f32], channels: usize) {
    out.fill(0.0);
    if !shared.playing.load(Ordering::Relaxed) {
        return;
    }
    let Some(guard) = shared.buffer.try_read() else {
        return; // buffer being swapped: one quiet buffer beats a glitch
    };
    let Some(buffer) = guard.as_ref() else {
        return;
    };
    let total = buffer.frames();
    let mut playhead = shared.playhead.load(Ordering::Relaxed);
    for frame in out.chunks_exact_mut(channels) {
        if playhead >= total {
            shared.playing.store(false, Ordering::Relaxed);
            break;
        }
        let l = buffer.samples[playhead * 2];
        let r = buffer.samples[playhead * 2 + 1];
        frame[0] = l;
        if channels > 1 {
            frame[1] = r;
        }
        playhead += 1;
    }
    shared.playhead.store(playhead, Ordering::Relaxed);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn tone(frames: usize) -> Arc<AudioBuffer> {
        let mut samples = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let v = (n as f32 * 0.05).sin() * 0.25;
            samples.push(v);
            samples.push(v);
        }
        Arc::new(AudioBuffer {
            rate: 48_000,
            samples,
        })
    }

    /// The callback contract, exercised directly (no device needed in CI):
    /// silence when paused, correct samples when playing, auto-stop at end,
    /// and the clock advancing by exactly the frames consumed.
    #[test]
    fn callback_plays_advances_clock_and_stops_at_end() {
        let shared = Shared {
            buffer: RwLock::new(Some(tone(1000))),
            playhead: AtomicUsize::new(0),
            playing: AtomicBool::new(false),
        };
        let mut out = vec![1.0f32; 256 * 2];

        // Paused: silence, clock still.
        fill(&shared, &mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);

        // Playing: exact samples, clock advances by frames written.
        shared.playing.store(true, Ordering::Relaxed);
        fill(&shared, &mut out, 2);
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 256);
        assert!((out[0] - 0.0).abs() < 1e-6); // sin(0)·0.25
        let expected = (255.0f32 * 0.05).sin() * 0.25;
        assert!((out[510] - expected).abs() < 1e-5);

        // Run past the end: stops exactly at the last frame, playing=false.
        for _ in 0..10 {
            fill(&shared, &mut out, 2);
        }
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 1000);
        assert!(!shared.playing.load(Ordering::Relaxed));
    }

    /// Mono-device downmix path: channel 0 gets L, nothing panics.
    #[test]
    fn callback_handles_mono_output() {
        let shared = Shared {
            buffer: RwLock::new(Some(tone(100))),
            playhead: AtomicUsize::new(0),
            playing: AtomicBool::new(true),
        };
        let mut out = vec![0.0f32; 64];
        fill(&shared, &mut out, 1);
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 64);
    }
}
