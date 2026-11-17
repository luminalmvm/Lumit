//! Racks on nodes: audio effects that run on a strip's whole sum, ahead of
//! the callback (docs/09-AUDIO.md).
//!
//! A rack is opened from a [`RackOpener`] the plan carries and runs as a
//! [`RackStream`], one block at a time. Its output reaches the mix two ways:
//!
//! - **Live**, the rack thread ([`crate::rack_thread`]) runs each racked
//!   node's stream ahead of the playhead and writes what comes out into the
//!   node's [`RackRing`], which the audio callback reads without waiting.
//! - **Offline** and on that rack thread, a [`RackPuller`] runs the streams
//!   itself as [`crate::plan::MixPlan::mix_block`] asks for their output.
//!
//! Either way a rack that delays its sound (a lookahead, say) is fed that
//! much ahead of where its output is wanted, so its delay is never heard.
//!
//! The ring is written by one thread and read by the callback with atomics
//! only. The puller runs on whichever thread owns it and may allocate.

use crate::block::{MixWork, RackSource, SCRATCH_FRAMES};
use crate::plan::MixPlan;
use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{fence, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// How many frames a rack processes at a time. The same as
/// `lumit_core::fx::audio_chain::AUDIO_BLOCK_FRAMES`, which this crate does
/// not depend on.
pub const AUDIO_BLOCK_FRAMES: usize = 512;

/// One opened rack, with its state. Fed one [`AUDIO_BLOCK_FRAMES`] block at
/// a time, in order.
pub trait RackStream: Send {
    /// How many frames late its output is.
    fn latency(&self) -> u32;
    /// How long it goes on sounding after its input stops, in frames.
    fn tail(&self) -> u32;
    /// Process one block of interleaved stereo `input` into `output`; `at` is
    /// the timeline frame of the input's first frame. False when it could
    /// not, and the block is then heard dry.
    fn process(&mut self, input: &[f32], output: &mut [f32], at: i64) -> bool;
    /// The stream as itself, so the opener that made it can reach its own
    /// type in [`RackOpener::update`]. Written `self` in every stream.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// What a plan carries to open a rack's stream.
pub trait RackOpener: Send + Sync {
    /// The rack's structure: which effects, in which order, which are on,
    /// and any plugin state. Not their parameter values. Equal for two
    /// openers of the same structure, so a stream is kept open across a plan
    /// swap and handed the new values through [`Self::update`] instead:
    /// dragging or keying a parameter never restarts the stream and never
    /// cuts a tail.
    fn identity(&self) -> u64;
    /// A fresh stream at `rate`. `offline` for an export, which may take as
    /// long as it needs. None when the rack will not open: its node is then
    /// heard dry.
    fn open(&self, rate: u32, offline: bool) -> Option<Box<dyn RackStream>>;
    /// Hand an open stream of the same identity this opener's parameter
    /// values, without restarting it. Answers whether it did. The default
    /// hands nothing, and the stream carries on as it was, which is right for
    /// an opener whose identity covers its values too. A stream whose delay
    /// changes with the new values is started again.
    fn update(&self, stream: &mut dyn RackStream) -> bool {
        let _ = stream;
        false
    }
}

/// A racked node's output waiting for the callback, by timeline frame.
///
/// One writer (the rack thread) and one reader (the callback). The writer
/// appends at the frame it has produced to; the reader copies what it needs
/// from the frames still in the ring. A restart (a seek, a new stream) moves
/// the generation on, and a read that saw the generation change while it
/// copied hands back nothing, so the sound of one moment is never played as
/// another's.
pub struct RackRing {
    /// Interleaved stereo as f32 bits, `capacity` frames.
    samples: Box<[AtomicU32]>,
    /// Frames the ring holds: a power of two.
    capacity: usize,
    /// The first frame still in the ring, and one past the last written.
    valid_from: AtomicI64,
    produced_to: AtomicI64,
    /// Odd while a restart is under way. Moved on by every restart.
    gen: AtomicU64,
}

/// What a read saw when it began.
struct Opened {
    gen: u64,
    frames: usize,
}

impl RackRing {
    /// A ring of at least a second at `rate`, empty.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        let capacity = (rate as usize).max(4 * SCRATCH_FRAMES).next_power_of_two();
        Self {
            samples: (0..capacity * 2).map(|_| AtomicU32::new(0)).collect(),
            capacity,
            valid_from: AtomicI64::new(0),
            produced_to: AtomicI64::new(0),
            gen: AtomicU64::new(0),
        }
    }

    /// How many frames it holds.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The first frame still in the ring.
    #[must_use]
    pub fn valid_from(&self) -> i64 {
        self.valid_from.load(Ordering::Acquire)
    }

    /// One past the last frame written.
    #[must_use]
    pub fn produced_to(&self) -> i64 {
        self.produced_to.load(Ordering::Acquire)
    }

    /// The writer starts again at `frame`: everything in the ring is gone.
    pub fn restart(&self, frame: i64) {
        self.gen.fetch_add(1, Ordering::Relaxed);
        fence(Ordering::Release);
        self.valid_from.store(frame, Ordering::Relaxed);
        self.produced_to.store(frame, Ordering::Relaxed);
        self.gen.fetch_add(1, Ordering::Release);
    }

    /// The writer adds interleaved stereo `samples` at frame `at`, which must
    /// be where it got to ([`Self::produced_to`]), and no further than the
    /// ring's length less two buffers past `reader`, the frame the callback is
    /// reading at. Answers how many frames went in.
    pub fn write(&self, at: i64, samples: &[f32], reader: i64) -> usize {
        if at != self.produced_to.load(Ordering::Relaxed) {
            return 0;
        }
        let room = reader + (self.capacity - 2 * SCRATCH_FRAMES) as i64 - at;
        let frames = (samples.len() / 2).min(room.max(0) as usize);
        if frames == 0 {
            return 0;
        }
        // The frames these overwrite leave the ring before they are touched.
        let floor = at + frames as i64 - self.capacity as i64;
        if floor > self.valid_from.load(Ordering::Relaxed) {
            self.valid_from.store(floor, Ordering::Relaxed);
            fence(Ordering::Release);
        }
        let mask = self.capacity - 1;
        for (n, frame) in samples.chunks_exact(2).take(frames).enumerate() {
            let slot = ((at as usize).wrapping_add(n) & mask) * 2;
            if let Some([l, r]) = self.samples.get(slot..slot + 2) {
                l.store(frame[0].to_bits(), Ordering::Relaxed);
                r.store(frame[1].to_bits(), Ordering::Relaxed);
            }
        }
        self.produced_to
            .store(at + frames as i64, Ordering::Release);
        frames
    }

    /// The reader copies up to `frames` frames from frame `at` into `out`
    /// (interleaved stereo). Answers how many, from `at` on; nought when `at`
    /// is not in the ring or a restart or the writer got in the way.
    pub fn read(&self, at: i64, frames: usize, out: &mut [f32]) -> usize {
        let Some(opened) = self.open(at, frames.min(out.len() / 2)) else {
            return 0;
        };
        self.copy(at, opened.frames, out);
        if self.close(&opened, at) {
            opened.frames
        } else {
            0
        }
    }

    /// A read's first step: what is there to copy.
    fn open(&self, at: i64, frames: usize) -> Option<Opened> {
        let gen = self.gen.load(Ordering::Acquire);
        if gen % 2 == 1 {
            return None;
        }
        let produced = self.produced_to.load(Ordering::Acquire);
        let from = self.valid_from.load(Ordering::Acquire);
        if at < from || at >= produced {
            return None;
        }
        Some(Opened {
            gen,
            frames: frames.min((produced - at) as usize),
        })
    }

    /// A read's second step: the copy.
    fn copy(&self, at: i64, frames: usize, out: &mut [f32]) {
        let mask = self.capacity - 1;
        for (n, frame) in out.chunks_exact_mut(2).take(frames).enumerate() {
            let slot = ((at as usize).wrapping_add(n) & mask) * 2;
            if let Some([l, r]) = self.samples.get(slot..slot + 2) {
                frame[0] = f32::from_bits(l.load(Ordering::Relaxed));
                frame[1] = f32::from_bits(r.load(Ordering::Relaxed));
            }
        }
    }

    /// A read's last step: whether what was copied is still what it was.
    fn close(&self, opened: &Opened, at: i64) -> bool {
        fence(Ordering::Acquire);
        self.gen.load(Ordering::Relaxed) == opened.gen
            && self.valid_from.load(Ordering::Relaxed) <= at
    }
}

/// One rack as the puller runs it.
#[derive(Default)]
struct Pull {
    /// The ring of the plan rack it runs, which is how a stream is found
    /// again in the next plan.
    ring: Option<Arc<RackRing>>,
    /// The identity it was opened for, None before it ever was.
    opened: Option<u64>,
    stream: Option<Box<dyn RackStream>>,
    latency: usize,
    /// Output frames from `out_from`, stereo.
    out: VecDeque<f32>,
    out_from: i64,
    /// Input frames from where the output has got to, stereo: what a block
    /// the stream will not process is heard as.
    dry: VecDeque<f32>,
    /// The mix that works out the node's input, kept so a duck on the way in
    /// carries on from block to block.
    work: Option<Box<MixWork>>,
    input: Vec<f32>,
    output: Vec<f32>,
}

impl Pull {
    /// One past the last output frame.
    fn produced(&self) -> i64 {
        self.out_from + (self.out.len() / 2) as i64
    }
}

/// Runs a plan's racks itself, as [`MixPlan::mix_block`] asks for their
/// output: an export, and the rack thread working out what to put in the
/// rings. A rack with a delay is fed that far ahead, so its output lands on
/// its own input's frame.
///
/// A request behind what it has kept, or more than a block past what it has
/// made, starts the rack again: run in from far enough back that its tail is
/// there. Offline the stream is opened afresh for it (racks have no reset,
/// and a fresh one is the same every run); live the open stream is run in
/// again, since opening a hosted plugin can take longer than a ring holds. A
/// rack whose identity changes is opened afresh either way.
///
/// Handed a new plan, it finds each rack's stream by the rack's ring, which
/// the builder hands an unchanged rack again ([`Self::adopt`]).
pub struct RackPuller {
    rate: u32,
    offline: bool,
    racks: Vec<Pull>,
    /// The plan the streams were last matched to.
    seen: u64,
    /// Frames of some clip that were due and not in memory while working out
    /// a rack's input. Only grows.
    pub missed: usize,
}

impl RackPuller {
    /// A puller for sound at `rate`; `offline` for an export.
    #[must_use]
    pub fn new(rate: u32, offline: bool) -> Self {
        Self {
            rate,
            offline,
            racks: Vec::new(),
            seen: 0,
            missed: 0,
        }
    }

    /// Take on `plan`, once per plan: each of its racks gets the stream
    /// already running for the same ring. Where the opener's identity is
    /// unchanged the stream is handed the new parameter values
    /// ([`RackOpener::update`]) and carries on; where it changed, the stream
    /// is opened afresh when next asked, from where it is asked. Streams
    /// whose rings are gone are dropped.
    pub fn adopt(&mut self, plan: &MixPlan) {
        if plan.id == self.seen {
            return;
        }
        self.seen = plan.id;
        let mut old = std::mem::take(&mut self.racks);
        self.racks = plan
            .racks
            .iter()
            .map(|rack| {
                let found = old
                    .iter()
                    .position(|p| p.ring.as_ref().is_some_and(|r| Arc::ptr_eq(r, &rack.ring)));
                let mut pull = found.map(|i| old.swap_remove(i)).unwrap_or_default();
                pull.ring = Some(Arc::clone(&rack.ring));
                if pull.opened == Some(rack.opener.identity()) {
                    if let Some(stream) = pull.stream.as_mut() {
                        let updated = rack.opener.update(stream.as_mut());
                        if updated && stream.latency() as usize != pull.latency {
                            pull.opened = None;
                        }
                    }
                }
                pull
            })
            .collect();
    }

    /// Rack `rack`'s delay in frames, as its stream reports it; nought
    /// before it is opened.
    #[must_use]
    pub fn latency(&self, rack: usize) -> u32 {
        self.racks
            .get(rack)
            .map_or(0, |p| u32::try_from(p.latency).unwrap_or(u32::MAX))
    }

    /// Set rack `r` to make output from `start`, with its tail run in and its
    /// delay fed ahead: opened afresh when offline or when it is not open for
    /// its identity.
    fn restart(&mut self, plan: &MixPlan, r: usize, start: i64) {
        let (Some(rack), Some(pull)) = (plan.racks.get(r), self.racks.get_mut(r)) else {
            return;
        };
        let identity = rack.opener.identity();
        if self.offline || pull.opened != Some(identity) {
            pull.stream = rack.opener.open(self.rate, self.offline);
            pull.opened = Some(identity);
        }
        let (latency, tail) = pull
            .stream
            .as_ref()
            .map_or((0, 0), |s| (s.latency() as usize, s.tail() as usize));
        let preroll =
            tail.min(self.rate as usize).div_ceil(AUDIO_BLOCK_FRAMES) * AUDIO_BLOCK_FRAMES;
        pull.latency = latency;
        pull.out.clear();
        pull.out_from = start - (preroll + latency) as i64;
        // The input under the first `latency` frames was never fed: silence.
        pull.dry.clear();
        pull.dry.resize(latency * 2, 0.0);
    }

    /// Run rack `r` one block further.
    fn pull_block(&mut self, plan: &MixPlan, r: usize) {
        let (Some(rack), Some(pull)) = (plan.racks.get(r), self.racks.get_mut(r)) else {
            return;
        };
        let node = rack.node;
        let block = AUDIO_BLOCK_FRAMES * 2;
        let produced = pull.produced();
        let at = produced + pull.latency as i64;
        let mut stream = pull.stream.take();
        let mut work = pull
            .work
            .take()
            .unwrap_or_else(|| Box::new(MixWork::new(self.rate)));
        let mut input = std::mem::take(&mut pull.input);
        let mut output = std::mem::take(&mut pull.output);
        input.clear();
        input.resize(block, 0.0);
        output.clear();
        output.resize(block, 0.0);
        // Frames before the timeline starts are silence.
        let before = (-at).clamp(0, AUDIO_BLOCK_FRAMES as i64) as usize;
        if let Some(rest) = input.get_mut(before * 2..) {
            if !rest.is_empty() {
                let from = (at + before as i64) as usize;
                let missed = plan.mix_block(from, rest, &mut work, self, None, Some(node));
                self.missed += missed;
            }
        }
        let processed = stream
            .as_mut()
            .is_some_and(|s| s.process(&input, &mut output, at));
        let Some(pull) = self.racks.get_mut(r) else {
            return;
        };
        pull.dry.extend(input.iter().copied());
        let dry = pull.dry.drain(..block.min(pull.dry.len()));
        if processed {
            pull.out.extend(output.iter().copied());
        } else {
            pull.out.extend(dry);
        }
        pull.stream = stream;
        pull.work = Some(work);
        pull.input = input;
        pull.output = output;
    }
}

impl RackSource for RackPuller {
    fn output(
        &mut self,
        plan: &MixPlan,
        rack: u16,
        start: usize,
        _input: &[f32],
        out: &mut [f32],
    ) -> bool {
        self.adopt(plan);
        let r = usize::from(rack);
        let Some(identity) = plan.racks.get(r).map(|p| p.opener.identity()) else {
            out.fill(0.0);
            return false;
        };
        if self.racks.len() <= r {
            self.racks.resize_with(r + 1, Pull::default);
        }
        let start = start as i64;
        let frames = (out.len() / 2) as i64;
        let behind = self.racks.get(r).is_none_or(|p| {
            p.opened != Some(identity)
                || start < p.out_from
                || start > p.produced() + AUDIO_BLOCK_FRAMES as i64
        });
        if behind {
            self.restart(plan, r, start);
        }
        // Keep a second behind the newest ask: a rack downstream with its own
        // delay asks for this one's output that much further on than the mix
        // does, and both are served from the one stream.
        if let Some(pull) = self.racks.get_mut(r) {
            let old = (start - i64::from(self.rate) - pull.out_from).max(0) as usize;
            let old = old.min(pull.out.len() / 2);
            pull.out.drain(..old * 2);
            pull.out_from += old as i64;
        }
        while self
            .racks
            .get(r)
            .is_some_and(|p| p.produced() < start + frames)
        {
            self.pull_block(plan, r);
        }
        let Some(pull) = self.racks.get(r) else {
            return false;
        };
        let skip = ((start - pull.out_from).max(0) as usize) * 2;
        for (o, s) in out.iter_mut().zip(pull.out.iter().skip(skip)) {
            *o = *s;
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::plan::{MixNode, PlacedClip, PlanRack};
    use crate::stream::Source;
    use std::sync::Arc;

    fn level(v: f32, frames: usize) -> Vec<f32> {
        vec![v; frames * 2]
    }

    /// A restart while a read is under way, and the writer lapping a read
    /// that fell behind, both make the read hand back nothing rather than
    /// sound from another moment. The steps of the read are interleaved with
    /// the writer's by hand.
    #[test]
    fn a_restart_mid_read_never_hands_back_another_moment() {
        let ring = RackRing::new(8_000);
        ring.restart(0);
        assert_eq!(ring.write(0, &level(0.1, 1_000), 0), 1_000);

        let opened = ring.open(100, 200).unwrap();
        ring.restart(100);
        assert_eq!(ring.write(100, &level(0.9, 500), 100), 500);
        let mut out = level(0.0, 200);
        ring.copy(100, opened.frames, &mut out);
        assert!(!ring.close(&opened, 100), "the restart is seen");
        // Read again after it: all of it the new moment.
        assert_eq!(ring.read(100, 200, &mut out), 200);
        assert!(out.iter().all(|v| *v == 0.9));

        // A read left open while the writer goes all the way round.
        let opened = ring.open(150, 100).unwrap();
        let cap = ring.capacity() as i64;
        let mut at = 600;
        while at < 150 + cap + 100 {
            at += ring.write(at, &level(0.5, 1_000), at - 100) as i64;
        }
        ring.copy(150, opened.frames, &mut out);
        assert!(!ring.close(&opened, 150), "the lapped frames are seen");
        assert_eq!(ring.read(150, 100, &mut out), 0);
    }

    /// Written and read a block at a time well past the ring's length: every
    /// frame comes back as it went in, across every wrap.
    #[test]
    fn a_ring_reads_back_exactly_across_the_wrap() {
        let ring = RackRing::new(8_000);
        ring.restart(-37);
        let frames = 3 * ring.capacity() + 501;
        let wave: Vec<f32> = (0..frames * 2).map(|n| n as f32 * 1e-4).collect();
        let (mut written, mut read) = (0usize, 0usize);
        let mut out = vec![0.0f32; 700 * 2];
        while read < frames {
            let at = written as i64 - 37;
            let chunk = &wave[written * 2..(written + 333).min(frames) * 2];
            written += ring.write(at, chunk, read as i64 - 37);
            let n = ring.read(read as i64 - 37, 700, &mut out);
            assert_eq!(&out[..n * 2], &wave[read * 2..(read + n) * 2]);
            read += n;
        }
    }

    /// A rack that delays its input by exactly its own latency.
    struct Delay {
        latency: u32,
        line: VecDeque<f32>,
    }

    impl RackStream for Delay {
        fn latency(&self) -> u32 {
            self.latency
        }
        fn tail(&self) -> u32 {
            0
        }
        fn process(&mut self, input: &[f32], output: &mut [f32], _at: i64) -> bool {
            self.line.extend(input.iter().copied());
            for o in output.iter_mut() {
                *o = self.line.pop_front().unwrap_or(0.0);
            }
            true
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct Opens(u32);

    impl RackOpener for Opens {
        fn identity(&self) -> u64 {
            u64::from(self.0)
        }
        fn open(&self, _rate: u32, _offline: bool) -> Option<Box<dyn RackStream>> {
            Some(Box::new(Delay {
                latency: self.0,
                line: std::iter::repeat_n(0.0, self.0 as usize * 2).collect(),
            }))
        }
    }

    /// An impulse through a rack 300 frames late comes out of the mix on its
    /// own frame, and through two in series (300 and 700) the same: the
    /// puller feeds each rack its delay ahead. Only the limiter's look ahead,
    /// which the clock takes off, is between the two.
    #[test]
    fn racks_with_latency_come_out_on_their_own_frame() {
        let rate = 48_000;
        let mut impulse = vec![0.0f32; 4_000 * 2];
        impulse[0] = 0.5;
        impulse[1] = 0.5;
        let clip = PlacedClip {
            source: Source::whole(Arc::new(lumit_media::audio::AudioBuffer {
                rate,
                samples: impulse,
            })),
            start_frame: 1_000,
            src_start: 0,
            len: 4_000,
            gain: [1.0, 1.0],
            envelope: None,
            node: 0,
            silent: false,
        };
        let rack = |node: u16, latency: u32| PlanRack {
            node,
            opener: Arc::new(Opens(latency)),
            ring: Arc::new(RackRing::new(rate)),
        };
        let one = MixPlan {
            clips: vec![clip.clone()],
            nodes: vec![MixNode {
                rack: Some(0),
                ..MixNode::default()
            }],
            total_frames: 5_000,
            racks: vec![rack(0, 300)],
            ..MixPlan::default()
        };
        let two = MixPlan {
            clips: vec![clip],
            nodes: vec![
                MixNode {
                    rack: Some(0),
                    out: Some(1),
                    ..MixNode::default()
                },
                MixNode {
                    rack: Some(1),
                    ..MixNode::default()
                },
            ],
            total_frames: 5_000,
            racks: vec![rack(0, 300), rack(1, 700)],
            ..MixPlan::default()
        };
        for plan in [one, two] {
            let plan = plan.finish().unwrap();
            let mut work = MixWork::new(rate);
            let mut puller = RackPuller::new(rate, true);
            let mut out = vec![0.0f32; 3_000 * 2];
            for (n, chunk) in out.chunks_mut(480 * 2).enumerate() {
                plan.mix_block(n * 480, chunk, &mut work, &mut puller, None, None);
            }
            let at = 1_000 + work.latency() as usize;
            for (f, s) in out.chunks_exact(2).enumerate() {
                let want = if f == at { 0.5 } else { 0.0 };
                assert!(
                    (s[0] - want).abs() < 1e-6,
                    "{} racks, frame {f}: {}",
                    plan.racks.len(),
                    s[0]
                );
            }
        }
    }
}
