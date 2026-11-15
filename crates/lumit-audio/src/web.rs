//! The browser's sound card (flutter_ui/web/sound.js).
//!
//! The page has one thread and no callback of its own on the sound card's
//! schedule. So what the callback and the filling thread do on the desktop is
//! done here a turn at a time: the mix is made a little ahead of the ear and
//! written into a ring that an audio worklet plays from. The clock is still
//! the sound's: what has been mixed, less what the worklet has still to play.

use crate::{
    block, fill_ahead, fill_smooth, loudness, nanos, rack_thread, slow, AudioEngine, AudioError,
    Edge, Scrub, Shared,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
extern "C" {
    /// Open the page's output, and answer its rate.
    #[wasm_bindgen(catch, js_namespace = lumitSound, js_name = open)]
    fn open() -> Result<f64, wasm_bindgen::JsValue>;
    /// How many frames have been written and not played yet.
    #[wasm_bindgen(js_namespace = lumitSound, js_name = queued)]
    fn queued() -> u32;
    /// Interleaved stereo, to play after what is queued.
    #[wasm_bindgen(js_namespace = lumitSound, js_name = write)]
    fn write(samples: &[f32]);
    /// Drop what is queued.
    #[wasm_bindgen(js_namespace = lumitSound, js_name = flush)]
    fn flush();
}

/// How far ahead of the ear the mix is made, in seconds. Playing, it has to
/// outlast the longest the page is busy with something else. Stopped, it is
/// how late a scrub is heard.
// ponytail: fixed. Make it follow how long the page's turns really stall if
// a slow machine gaps.
const PLAY_AHEAD_S: f64 = 0.15;
const STOPPED_AHEAD_S: f64 = 0.05;

/// How many blocks a turn decodes at most. Each is a copy out of memory here.
const FILLS_A_TURN: usize = 8;

/// What the callback keeps between buffers, kept between turns here.
pub(crate) struct Output {
    scrub: Scrub,
    edge: Edge,
    stretch: slow::Stretch,
    work: block::MixWork,
    loudness: loudness::Loudness,
    buffer: Vec<f32>,
    /// What the rack thread does on the desktop, done here a turn at a time.
    racks: rack_thread::RackThread,
}

impl AudioEngine {
    /// Open the page's output. There is only the one, so `wanted` is not asked.
    pub fn new_on(_wanted: Option<&str>) -> Result<Self, AudioError> {
        let rate = open().map_err(|_| AudioError::NoDevice)? as u32;
        Ok(Self {
            output: std::cell::RefCell::new(Output {
                scrub: Scrub::default(),
                edge: Edge::default(),
                stretch: slow::Stretch::new(rate),
                work: block::MixWork::new(rate).owning_live(),
                loudness: loudness::Loudness::new(rate),
                buffer: Vec::new(),
                racks: rack_thread::RackThread::new(rate),
            }),
            shared: Arc::new(Shared::new(None)),
            device_rate: rate,
            channels: 2,
            // Waking them does nothing: the filling is done on every turn.
            filler: std::thread::current(),
            racks: std::thread::current(),
        })
    }

    /// One turn of the sound: decode what is due, then top the ring up to
    /// [`PLAY_AHEAD_S`] ahead of the ear and set the clock by what is in it.
    pub fn turn(&self) {
        let shared = &*self.shared;
        for _ in 0..FILLS_A_TURN {
            if !fill_ahead(shared, self.device_rate) {
                break;
            }
        }
        let mut output = self.output.borrow_mut();
        let Output {
            scrub,
            edge,
            stretch,
            work,
            loudness,
            buffer,
            racks,
        } = &mut *output;
        for _ in 0..FILLS_A_TURN {
            if !rack_thread::rack_step(shared, racks) {
                break;
            }
        }
        let playing = shared.playing.load(Ordering::Relaxed);
        let playhead = shared.playhead.load(Ordering::Relaxed);
        let slow = shared.slow.load(Ordering::Relaxed).max(1) as usize;
        let mut ahead = queued() as usize;
        let at_end = shared
            .plan
            .read()
            .as_ref()
            .is_some_and(|plan| playhead >= plan.total_frames);
        // A start, a stop or a seek is heard now, not after what is queued.
        // Running off the end is not a stop: the end is still to be heard.
        if (playing != edge.playing || (playing && playhead != edge.next)) && !at_end {
            flush();
            // Stopped where the ear had got to, not where the mix had.
            if edge.playing && !playing && playhead == edge.next {
                shared
                    .playhead
                    .store(playhead.saturating_sub(ahead / slow), Ordering::Relaxed);
            }
            ahead = 0;
        }
        let rate = f64::from(self.device_rate);
        let wanted = if playing {
            PLAY_AHEAD_S
        } else {
            STOPPED_AHEAD_S
        } * rate;
        let frames = (wanted as usize).saturating_sub(ahead);
        if frames > 0 {
            buffer.resize(frames * 2, 0.0);
            fill_smooth(shared, scrub, edge, stretch, work, loudness, buffer, 2);
            write(buffer);
            ahead += frames;
        }
        // The clock runs on from what the worklet has played, up to where the
        // mix is. How late the page's output is heard after that is the Audio
        // offset's to make up, as on the desktop.
        shared.step.store(ahead / slow, Ordering::Relaxed);
        shared.stepped_at.store(nanos(), Ordering::Relaxed);
    }
}
