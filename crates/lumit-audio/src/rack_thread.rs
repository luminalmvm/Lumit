//! The rack thread (`lumit-audio-racks`): keeps each racked node's ring
//! filled ahead of the playhead, so the callback finds the rack's output
//! waiting for it (docs/09-AUDIO.md).
//!
//! It runs beside the filling thread at normal priority, and may allocate
//! and wait; the callback never waits for it. A rack that cannot keep up is
//! heard as silence on its node, counted as starved frames, never as a late
//! callback. On the page, which has one thread, the same step runs a few
//! times a turn.
//!
//! Each step it takes the current plan, matches its racks to the streams it
//! already runs (by ring, through [`RackPuller::adopt`]), and tops each ring
//! up a block toward its target: [`RACK_AHEAD_MS`] past the playhead, and
//! further by the delays of the racks downstream, so those can pull what
//! they need from it. Upstream racks go first. A playhead outside what a
//! ring holds (a seek, a loop, a new comp) starts that ring again, with the
//! rack run in from before it so its tail is there: at the playhead when
//! stopped, and while playing far enough past it that the thread is there
//! first (twice what the last start took), the node silent until then.

use crate::block::RackSource;
use crate::plan::MixPlan;
use crate::rack::{RackPuller, AUDIO_BLOCK_FRAMES};
use crate::Shared;
use std::sync::atomic::Ordering;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

/// How far past the playhead each ring is kept filled, in milliseconds.
pub const RACK_AHEAD_MS: i64 = 250;

/// How long the thread rests once every ring is full. Swaps, seeks and play
/// wake it sooner.
#[cfg(not(target_arch = "wasm32"))]
const RACK_REST: std::time::Duration = std::time::Duration::from_millis(5);

/// What the rack thread keeps from step to step.
pub(crate) struct RackThread {
    rate: u32,
    /// Runs the streams and works out each rack's input.
    puller: RackPuller,
    /// Per rack, how much further than [`RACK_AHEAD_MS`] its ring is filled:
    /// the longest run of delays of the racks downstream of it.
    lead: Vec<i64>,
    /// The plan's racks, upstream first.
    order: Vec<usize>,
    /// One block of a rack's output on its way to the ring.
    block: Vec<f32>,
    /// How far the playhead moved during the last step that started a ring
    /// again while playing: how long that takes.
    cost: i64,
    /// How far past the playhead the rings last started again. A playing
    /// playhead no further than this short of a ring is waiting for it.
    reach: i64,
}

impl RackThread {
    /// For sound at `rate`.
    pub(crate) fn new(rate: u32) -> Self {
        Self {
            rate,
            puller: RackPuller::new(rate, false),
            lead: Vec::new(),
            order: Vec::new(),
            block: Vec::new(),
            cost: 0,
            reach: 0,
        }
    }

    /// Put the plan's racks upstream first and work out each one's lead.
    fn plan_order(&mut self, plan: &MixPlan) {
        let count = plan.racks.len();
        self.order.clear();
        self.order.extend(0..count);
        self.order
            .sort_by_key(|&r| plan.racks.get(r).map_or(0, |rack| rack.node));
        self.lead.clear();
        self.lead.resize(count, 0);
        // Downstream first, so a rack's lead is whole before it is passed on
        // to the racks that feed it.
        for &d in self.order.iter().rev() {
            let reach = self.lead.get(d).copied().unwrap_or(0) + i64::from(self.puller.latency(d));
            for &a in plan.ancestors.get(d).into_iter().flatten() {
                let rack = plan.nodes.get(usize::from(a)).and_then(|n| n.rack);
                if let Some(lead) = rack.and_then(|r| self.lead.get_mut(usize::from(r))) {
                    *lead = (*lead).max(reach);
                }
            }
        }
    }
}

/// Start the rack thread.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn(shared: Arc<Shared>, rate: u32) -> std::io::Result<std::thread::Thread> {
    std::thread::Builder::new()
        .name("lumit-audio-racks".into())
        .spawn(move || {
            let mut racks = RackThread::new(rate);
            while !shared.closed.load(Ordering::Relaxed) {
                if !rack_step(&shared, &mut racks) {
                    std::thread::park_timeout(RACK_REST);
                }
            }
        })
        .map(|handle| handle.thread().clone())
}

/// **One step of the rack thread**: each racked node's ring a block nearer
/// its target. `true` when anything was written, which means there may be
/// more and it should be called again at once.
pub(crate) fn rack_step(shared: &Shared, racks: &mut RackThread) -> bool {
    let plan = shared.plan.read().clone();
    let Some(plan) = plan else {
        racks.puller = RackPuller::new(racks.rate, false);
        return false;
    };
    racks.puller.adopt(&plan);
    if plan.racks.is_empty() {
        return false;
    }
    racks.plan_order(&plan);
    let playhead = i64::try_from(shared.playhead.load(Ordering::Relaxed)).unwrap_or(i64::MAX);
    let playing = shared.playing.load(Ordering::Relaxed);
    let ahead = i64::from(racks.rate) * RACK_AHEAD_MS / 1000;
    let block = AUDIO_BLOCK_FRAMES as i64;
    let reach = if playing {
        (2 * racks.cost).max(block)
    } else {
        0
    };
    let RackThread {
        puller,
        lead,
        order,
        block: buffer,
        reach: waited,
        ..
    } = racks;
    let mut wrote = false;
    let mut restarted = false;
    for &r in order.iter() {
        let (Some(rack), Ok(index)) = (plan.racks.get(r), u16::try_from(r)) else {
            continue;
        };
        let ring = &rack.ring;
        let valid = ring.valid_from();
        let waiting = playing && valid - playhead <= *waited;
        let restart = (playhead < valid && !waiting) || playhead > ring.produced_to() + block;
        if restart {
            ring.restart(playhead.saturating_add(reach));
            restarted = true;
        }
        let at = ring.produced_to();
        let mut target = playhead + ahead + lead.get(r).copied().unwrap_or(0);
        if restart {
            // Run in now, so the time it takes is measured and the ring is
            // ready before the playhead gets there.
            target = target.max(at + block);
        }
        let frames = (target - at).clamp(0, block) as usize;
        let Ok(from) = usize::try_from(at) else {
            continue;
        };
        if frames == 0 {
            continue;
        }
        buffer.clear();
        buffer.resize(frames * 2, 0.0);
        puller.output(&plan, index, from, &[], buffer);
        wrote |= ring.write(at, buffer, playhead) > 0;
    }
    if restarted {
        racks.reach = reach;
        if playing {
            let now = i64::try_from(shared.playhead.load(Ordering::Relaxed)).unwrap_or(i64::MAX);
            racks.cost = now.saturating_sub(playhead).max(0);
        }
    }
    wrote
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::block::{MixWork, RingSource};
    use crate::plan::{MixNode, PlacedClip, PlanRack};
    use crate::rack::{RackOpener, RackRing, RackStream};
    use crate::stream::Source;
    use lumit_media::audio::AudioBuffer;
    use std::any::Any;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// A rack that turns its input upside down, with a second's tail. Each
    /// block it runs while playing takes as long as 200 frames of playback.
    struct Invert {
        shared: Arc<Shared>,
        opens: Arc<AtomicUsize>,
    }

    impl RackStream for Invert {
        fn latency(&self) -> u32 {
            0
        }
        fn tail(&self) -> u32 {
            8_000
        }
        fn process(&mut self, input: &[f32], output: &mut [f32], _at: i64) -> bool {
            if self.shared.playing.load(Ordering::Relaxed) {
                self.shared.playhead.fetch_add(200, Ordering::Relaxed);
            }
            for (o, i) in output.iter_mut().zip(input) {
                *o = -i;
            }
            true
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    impl RackOpener for Invert {
        fn identity(&self) -> u64 {
            1
        }
        fn open(&self, _rate: u32, _offline: bool) -> Option<Box<dyn RackStream>> {
            self.opens.fetch_add(1, Ordering::Relaxed);
            Some(Box::new(Invert {
                shared: Arc::clone(&self.shared),
                opens: Arc::clone(&self.opens),
            }))
        }
    }

    /// A clip through a strip into a racked bus: once the rack thread has
    /// caught up, the callback hears the rack's output on the clip's own
    /// frames. A seek far off starts the ring again there, and nothing of the
    /// old window is read back. Playing, a seek whose start takes longer
    /// than a block of playback still catches the playhead, and the rack is
    /// never opened again for it.
    #[test]
    fn the_rack_thread_fills_the_ring_and_a_seek_starts_it_again() {
        let rate = 8_000u32;
        let frames = rate as usize * 4;
        let wave = |n: usize| (n as f32 * 0.013).sin() * 0.5;
        let samples = (0..frames).flat_map(|n| [wave(n), wave(n)]).collect();
        let clip = PlacedClip {
            source: Source::whole(Arc::new(AudioBuffer { rate, samples })),
            start_frame: 0,
            src_start: 0,
            len: frames,
            gain: [1.0, 1.0],
            envelope: None,
            node: 0,
            silent: false,
        };
        let shared = Arc::new(Shared::new(None));
        let opens = Arc::new(AtomicUsize::new(0));
        let plan = MixPlan {
            clips: vec![clip],
            nodes: vec![
                MixNode {
                    out: Some(1),
                    ..MixNode::default()
                },
                MixNode {
                    rack: Some(0),
                    ..MixNode::default()
                },
            ],
            total_frames: frames,
            racks: vec![PlanRack {
                node: 1,
                opener: Arc::new(Invert {
                    shared: Arc::clone(&shared),
                    opens: Arc::clone(&opens),
                }),
                ring: Arc::new(RackRing::new(rate)),
            }],
            ..MixPlan::default()
        }
        .finish()
        .unwrap();
        let plan = Arc::new(plan);
        *shared.plan.write() = Some(Arc::clone(&plan));
        let mut racks = RackThread::new(rate);
        let mut work = MixWork::new(rate);
        work.set_limiter(false);
        let mut out = vec![0.0f32; 1_000 * 2];
        let heard = |at: usize, work: &mut MixWork, out: &mut [f32]| {
            let mut rings = RingSource::default();
            plan.mix_block(at, out, work, &mut rings, None, None);
            rings.missed + rings.waiting
        };

        shared.playhead.store(300, Ordering::Relaxed);
        while rack_step(&shared, &mut racks) {}
        assert_eq!(heard(300, &mut work, &mut out), 0);
        for (k, f) in out.chunks_exact(2).enumerate() {
            assert!((f[0] + wave(300 + k)).abs() < 1e-6, "frame {}", 300 + k);
        }

        // A seek far off: nothing there until the thread has been round.
        let far = 20_000;
        shared.playhead.store(far, Ordering::Relaxed);
        assert_eq!(heard(far, &mut work, &mut out), 1_000);
        assert!(out.iter().all(|s| *s == 0.0));
        while rack_step(&shared, &mut racks) {}
        assert_eq!(heard(far, &mut work, &mut out), 0);
        for (k, f) in out.chunks_exact(2).enumerate() {
            assert!((f[0] + wave(far + k)).abs() < 1e-6, "frame {}", far + k);
        }
        // The old window is gone from the ring.
        let ring = &plan.racks[0].ring;
        assert_eq!(ring.valid_from(), far as i64);
        assert_eq!(ring.read(300, 1_000, &mut out), 0);

        // Playing, a seek back: the rack's second of tail takes 17 blocks to
        // run in, far longer than a block of playback.
        shared.playing.store(true, Ordering::Relaxed);
        shared.playhead.store(5_000, Ordering::Relaxed);
        let mut steps = 0;
        loop {
            while rack_step(&shared, &mut racks) {
                steps += 1;
                assert!(steps < 1_000, "the ring starts again for ever");
            }
            let at = shared.playhead.load(Ordering::Relaxed);
            assert!(at < 25_000, "the ring never caught the playhead");
            if heard(at, &mut work, &mut out) == 0 {
                for (k, f) in out.chunks_exact(2).enumerate() {
                    assert!((f[0] + wave(at + k)).abs() < 1e-6, "frame {}", at + k);
                }
                break;
            }
            // The callback plays on.
            shared.playhead.store(at + 1_000, Ordering::Relaxed);
        }
        assert_eq!(opens.load(Ordering::Relaxed), 1);
        *shared.plan.write() = None;
    }
}
