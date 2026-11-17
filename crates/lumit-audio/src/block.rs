//! The mix itself, a block at a time: clips into their nodes, nodes through
//! their racks, ducks, sends and faders into later nodes, and the master
//! through its fader and the limiter (docs/09-AUDIO.md).
//!
//! The audio callback, the rack thread and an export all mix through
//! [`MixPlan::mix_block`], each with a [`MixWork`] of its own: the buffers and
//! the state that runs on from one block to the next. Everything is made when
//! the `MixWork` is, so on the callback a block allocates nothing, takes no
//! lock and never waits: a clip whose sound is not in memory is silence and
//! counted, and a rack's output comes from wherever the caller's
//! [`RackSource`] says.

use crate::duck::{self, Follower};
use crate::limiter::Limiter;
use crate::live::{LiveFader, FOLLOW, HOLD, TRIM};
use crate::meter::{self, MeterAccs};
use crate::plan::{Fader, MixPlan, PlacedClip, MAX_NODES, MAX_SENDS, NONE};
use crate::rack::AUDIO_BLOCK_FRAMES;

/// How many frames a block is mixed in at most. A longer stretch is mixed as
/// several.
pub const SCRATCH_FRAMES: usize = 1024;

/// A master limiter reduction past this (0.1 dB) lights the master's clip
/// lamp.
const LAMP_GAIN: f32 = 0.988_553_1;

/// Where a racked node's output comes from.
pub trait RackSource {
    /// Rack `rack`'s output for the frames from `start` into `out`, given the
    /// node's `input` for the same frames (interleaved stereo, both). False
    /// when there is none, and the node is silent.
    fn output(
        &mut self,
        plan: &MixPlan,
        rack: u16,
        start: usize,
        input: &[f32],
        out: &mut [f32],
    ) -> bool;
}

/// The callback's racks: what the rack thread has put in each node's ring.
/// Frames not there yet are silence, and counted.
#[derive(Debug, Default)]
pub struct RingSource {
    /// Frames of rack output that were due and not in the ring. Only grows.
    pub missed: usize,
    /// Frames silent by design: the ring was started again ahead of them
    /// after a seek, or has not yet been started where they are, so the
    /// rack is running in rather than falling behind. Only grows.
    pub waiting: usize,
}

impl RackSource for RingSource {
    fn output(
        &mut self,
        plan: &MixPlan,
        rack: u16,
        start: usize,
        _input: &[f32],
        out: &mut [f32],
    ) -> bool {
        let frames = out.len() / 2;
        let ring = plan.racks.get(usize::from(rack)).map(|r| &r.ring);
        let at = start as i64;
        let got = ring.map_or(0, |ring| ring.read(at, frames, out));
        if let Some(rest) = out.get_mut(got * 2..) {
            rest.fill(0.0);
        }
        let running_in = ring.is_some_and(|ring| {
            at < ring.valid_from() || at > ring.produced_to() + AUDIO_BLOCK_FRAMES as i64
        });
        if running_in {
            self.waiting += frames - got;
        } else {
            self.missed += frames - got;
        }
        got > 0
    }
}

/// The scrub's racks: none. Each racked node is heard as it goes in.
#[derive(Debug, Default)]
pub struct DrySource;

impl RackSource for DrySource {
    fn output(
        &mut self,
        _plan: &MixPlan,
        _rack: u16,
        _start: usize,
        input: &[f32],
        out: &mut [f32],
    ) -> bool {
        out.copy_from_slice(input);
        true
    }
}

/// A fader's override as one mix has followed it.
#[derive(Clone, Copy, Debug, Default)]
struct Glide {
    /// Held last block, at `last`, as a trim or not: a new held value of the
    /// same kind ramps from there.
    holding: bool,
    trim: bool,
    last: [f32; 2],
    /// Gliding back to the keys: the release it began at, and how far in.
    gliding: bool,
    epoch: u32,
    done: u32,
}

/// What one node keeps from block to block.
#[derive(Clone, Copy, Debug, Default)]
struct NodeState {
    duck: Follower,
    glide: Glide,
}

/// One mix's buffers and running state. The callback owns one, and so do the
/// rack thread and an export; none may be shared, since each block carries
/// on from the one before.
pub struct MixWork {
    /// A [`SCRATCH_FRAMES`] stereo buffer per node.
    scratch: Box<[f32]>,
    /// A racked node's input while its rack is asked for the output.
    input: Box<[f32]>,
    /// A fader's gain per frame.
    gains: Box<[f32]>,
    /// Which nodes this block mixes.
    play: Box<[bool]>,
    /// Per node; `spare` is what a new plan's are made in.
    nodes: Box<[NodeState]>,
    spare: Box<[NodeState]>,
    master: Glide,
    /// Per send, by its override's index; `spare_sends` is what a new plan's
    /// are made in.
    sends: Box<[Glide]>,
    spare_sends: Box<[Glide]>,
    /// Whether this mix is the one heard, the callback's, and so the only one
    /// that may end a glide in the plan's [`crate::live::LiveBank`]. The rack
    /// thread and the peaks run ahead of it and would cut it short.
    owns_live: bool,
    limiter: Limiter,
    /// Whether the master goes through the limiter, or a plain clamp.
    limit: bool,
    /// The plan these states belong to.
    seen: u64,
    /// The rate the mix runs at.
    rate: u32,
    /// What the jog reads the mix into before it is played at its speed
    /// ([`crate::jog`]): a chunk's worth at four times speed.
    jog: Box<[f32]>,
}

impl MixWork {
    /// A mix's buffers for sound at `rate`, made here and never again.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        Self {
            scratch: vec![0.0; MAX_NODES * SCRATCH_FRAMES * 2].into_boxed_slice(),
            input: vec![0.0; SCRATCH_FRAMES * 2].into_boxed_slice(),
            gains: vec![0.0; SCRATCH_FRAMES * 2].into_boxed_slice(),
            play: vec![false; MAX_NODES].into_boxed_slice(),
            nodes: vec![NodeState::default(); MAX_NODES].into_boxed_slice(),
            spare: vec![NodeState::default(); MAX_NODES].into_boxed_slice(),
            master: Glide::default(),
            sends: vec![Glide::default(); MAX_SENDS].into_boxed_slice(),
            spare_sends: vec![Glide::default(); MAX_SENDS].into_boxed_slice(),
            owns_live: false,
            limiter: Limiter::new(rate),
            limit: true,
            seen: 0,
            rate,
            jog: vec![0.0; crate::jog::JOG_FRAMES * 2].into_boxed_slice(),
        }
    }

    /// The same mix as the one heard: it ends the glides it finishes
    /// ([`Self::owns_live`]).
    #[must_use]
    pub(crate) fn owning_live(mut self) -> Self {
        self.owns_live = true;
        self
    }

    /// The rate the mix runs at.
    #[must_use]
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Lend the jog's buffer out, so the mix can run while the jog holds it.
    /// Hand it back with [`Self::return_jog`]. Allocates nothing.
    pub(crate) fn lend_jog(&mut self) -> Box<[f32]> {
        std::mem::take(&mut self.jog)
    }

    /// Take the jog's buffer back.
    pub(crate) fn return_jog(&mut self, jog: Box<[f32]>) {
        self.jog = jog;
    }

    /// How many frames late the master is: the limiter's look ahead while it
    /// is on, else nought.
    #[must_use]
    pub fn latency(&self) -> u32 {
        if self.limit {
            self.limiter.latency()
        } else {
            0
        }
    }

    /// Put the master through the limiter (the default) or, off, through a
    /// plain clamp at the ceiling with no delay: what a mix read out of order
    /// (a scrub, slowed playback) needs, since the limiter looks ahead.
    pub fn set_limiter(&mut self, on: bool) {
        self.limit = on;
    }

    /// Take on `plan`: when it is not the plan of the last block, each node's
    /// state moves to where [`MixPlan::carry`] says the node now is, and each
    /// send's to where the plan's bank says it was seeded from.
    fn adopt(&mut self, plan: &MixPlan) {
        if plan.id == self.seen {
            return;
        }
        for (n, state) in self.spare.iter_mut().enumerate() {
            *state = plan
                .carry
                .get(n)
                .filter(|&&o| o != NONE)
                .and_then(|&o| self.nodes.get(usize::from(o)))
                .copied()
                .unwrap_or_default();
        }
        std::mem::swap(&mut self.nodes, &mut self.spare);
        for (s, glide) in self.spare_sends.iter_mut().enumerate() {
            *glide = plan
                .live
                .send_carried(s)
                .and_then(|o| self.sends.get(usize::from(o)))
                .copied()
                .unwrap_or_default();
        }
        std::mem::swap(&mut self.sends, &mut self.spare_sends);
        self.seen = plan.id;
    }
}

/// `fader`'s gain for each of the frames `gains` holds (stereo), from
/// timeline frame `start`: its override where one is held or gliding (times
/// its keys for a trim), else its keys or constant. Only the mix that `owns`
/// the bank ends a glide there; any other holds at its end until it does.
fn fill_gains(
    fader: &Fader,
    live: Option<&LiveFader>,
    glide: &mut Glide,
    owns: bool,
    start: usize,
    gains: &mut [f32],
) {
    let frames = gains.len() / 2;
    let mode = live.map_or(FOLLOW, LiveFader::mode);
    let Some(live) = live.filter(|_| mode != FOLLOW) else {
        glide.holding = false;
        glide.gliding = false;
        for (k, g) in gains.chunks_exact_mut(2).enumerate() {
            g.copy_from_slice(&fader.at(start + k));
        }
        return;
    };
    let held = live.gains();
    if mode == HOLD || mode == TRIM {
        let trim = mode == TRIM;
        // A move while held ramps across the block rather than stepping.
        let from = if glide.holding && glide.trim == trim {
            glide.last
        } else {
            held
        };
        for (k, g) in gains.chunks_exact_mut(2).enumerate() {
            let t = (k + 1) as f32 / frames as f32;
            g[0] = from[0] + (held[0] - from[0]) * t;
            g[1] = from[1] + (held[1] - from[1]) * t;
            if trim {
                let keys = fader.at(start + k);
                g[0] *= keys[0];
                g[1] *= keys[1];
            }
        }
        glide.holding = true;
        glide.trim = trim;
        glide.gliding = false;
        glide.last = held;
        return;
    }
    let times = live.trims();
    let epoch = live.epoch();
    if !glide.gliding || glide.epoch != epoch {
        glide.gliding = true;
        glide.epoch = epoch;
        glide.done = 0;
    }
    glide.holding = false;
    let length = live.glide_frames().max(1);
    for (k, g) in gains.chunks_exact_mut(2).enumerate() {
        let t = ((glide.done as usize + k) as f32 / length as f32).min(1.0);
        let keys = fader.at(start + k);
        // From what was heard: the held gains, or the trimmed keys.
        let from = if times {
            [held[0] * keys[0], held[1] * keys[1]]
        } else {
            held
        };
        g[0] = from[0] + (keys[0] - from[0]) * t;
        g[1] = from[1] + (keys[1] - from[1]) * t;
    }
    glide.done = glide.done.saturating_add(frames as u32);
    if glide.done >= length && owns {
        live.finish_glide(epoch);
        glide.gliding = false;
    }
}

/// Add `clip`'s part of the frames from `start` into `dst` (stereo), through
/// its gain. Answers how many of its frames were due and not in memory.
fn add_clip(clip: &PlacedClip, start: usize, dst: &mut [f32]) -> usize {
    let frames = dst.len() / 2;
    let Some((at, first, count)) = MixPlan::sounding(clip, start, start + frames) else {
        return 0;
    };
    clip.source
        .read_runs(clip.src_start + first, count, |into, run| {
            let Some(d) = dst.get_mut((at + into) * 2..) else {
                return;
            };
            for (k, (s, o)) in run.chunks_exact(2).zip(d.chunks_exact_mut(2)).enumerate() {
                let g = clip
                    .envelope
                    .as_ref()
                    .map_or(clip.gain, |e| e.gain_at(first + into + k));
                o[0] += s[0] * g[0];
                o[1] += s[1] * g[1];
            }
        })
}

impl MixPlan {
    /// **The mix.** The frames from timeline frame `start` into `out`
    /// (interleaved stereo, overwritten), through every node and the master,
    /// with each node's level metered into its slot and the master's into
    /// [`meter::MASTER`] when `acc` is given. Answers how many frames of some
    /// clip were due and not in memory, which were mixed as silence.
    ///
    /// With `stop_at` set to a node, only that node and what feeds it are
    /// mixed, and `out` is the node's input (its sum before its rack): what a
    /// rack is fed.
    ///
    /// Allocation-free and lock-free. With the limiter on (see
    /// [`MixWork::set_limiter`]) the master is [`MixWork::latency`] frames
    /// late.
    // ponytail: every clip is tested per call, which is 2,000 compares a
    // buffer for a long cut and nothing beside the mixing. An index of clips
    // by start is the upgrade if a plan ever holds hundreds of thousands.
    pub fn mix_block(
        &self,
        start: usize,
        out: &mut [f32],
        work: &mut MixWork,
        racks: &mut dyn RackSource,
        mut acc: Option<&mut MeterAccs>,
        stop_at: Option<u16>,
    ) -> usize {
        work.adopt(self);
        let mut missed = 0;
        let mut at = start;
        for chunk in out.chunks_mut(SCRATCH_FRAMES * 2) {
            missed += self.mix_chunk(at, chunk, work, racks, acc.as_deref_mut(), stop_at);
            at += chunk.len() / 2;
        }
        missed
    }

    /// [`Self::mix_block`] for at most [`SCRATCH_FRAMES`] frames.
    fn mix_chunk(
        &self,
        start: usize,
        out: &mut [f32],
        work: &mut MixWork,
        racks: &mut dyn RackSource,
        mut acc: Option<&mut MeterAccs>,
        stop_at: Option<u16>,
    ) -> usize {
        out.fill(0.0);
        let len = (out.len() / 2) * 2;
        let out = &mut out[..len];
        let count = self.nodes.len().min(MAX_NODES);
        let stride = SCRATCH_FRAMES * 2;
        let MixWork {
            scratch,
            input,
            gains,
            play,
            nodes: states,
            master: master_glide,
            sends: send_glides,
            limiter,
            limit,
            owns_live,
            ..
        } = work;
        let owns = *owns_live;
        let gains = &mut gains[..len];

        // Which nodes are in play: all of them, or the one asked for and
        // what feeds it.
        play[..count].fill(stop_at.is_none());
        if let Some(stop) = stop_at.map(usize::from).filter(|&s| s < count) {
            play[stop] = true;
            let needs = self.nodes[stop]
                .rack
                .and_then(|r| self.ancestors.get(usize::from(r)));
            match needs {
                Some(list) => {
                    for &a in list {
                        if let Some(p) = play.get_mut(usize::from(a)) {
                            *p = true;
                        }
                    }
                }
                None => play[..stop].fill(true),
            }
        }
        for (n, own) in scratch.chunks_exact_mut(stride).take(count).enumerate() {
            if play[n] {
                own[..len].fill(0.0);
            }
        }

        let mut missed = 0;
        for clip in self.clips.iter().filter(|c| !c.silent) {
            let dst = if clip.node == NONE {
                if stop_at.is_some() {
                    continue;
                }
                &mut *out
            } else {
                let n = usize::from(clip.node);
                if n >= count || !play[n] {
                    continue;
                }
                &mut scratch[n * stride..n * stride + len]
            };
            missed += add_clip(clip, start, dst);
        }

        for (i, node) in self.nodes.iter().enumerate().take(count) {
            if !play[i] {
                continue;
            }
            let (before, rest) = scratch.split_at_mut(i * stride);
            let (own, after) = rest.split_at_mut(stride);
            let own = &mut own[..len];
            if stop_at.map(usize::from) == Some(i) {
                out.copy_from_slice(own);
                return missed;
            }
            if let Some(r) = node.rack {
                let input = &mut input[..len];
                input.copy_from_slice(own);
                if !racks.output(self, r, start, input, own) {
                    own.fill(0.0);
                }
            }
            let mut lowest = 1.0f32;
            if let Some(d) = &node.duck {
                let k = usize::from(d.key);
                if k < i && play[k] {
                    let key = &before[k * stride..k * stride + len];
                    lowest = duck::apply(d, &mut states[i].duck, key, own);
                }
            }
            let mut send = |pre: bool, own: &[f32], after: &mut [f32], gains: &mut [f32]| {
                for s in node.sends.iter().filter(|s| s.pre == pre && s.on) {
                    let to = usize::from(s.to);
                    if to <= i || to >= count || !play[to] {
                        continue;
                    }
                    let live = s.level.live.and_then(|l| self.live.send(usize::from(l)));
                    let mut spare = Glide::default();
                    let glide = s
                        .level
                        .live
                        .and_then(|l| send_glides.get_mut(usize::from(l)))
                        .unwrap_or(&mut spare);
                    fill_gains(&s.level, live, glide, owns, start, gains);
                    let at = (to - i - 1) * stride;
                    let dst = &mut after[at..at + len];
                    for ((d, v), g) in dst.iter_mut().zip(own.iter()).zip(gains.iter()) {
                        *d += v * g;
                    }
                }
            };
            send(true, own, after, gains);
            let live = node
                .fader
                .live
                .and_then(|l| self.live.fader(usize::from(l)));
            fill_gains(&node.fader, live, &mut states[i].glide, owns, start, gains);
            for (v, g) in own.iter_mut().zip(gains.iter()) {
                *v *= g;
            }
            if let Some(slot) = acc.as_deref_mut().and_then(|a| a.get_mut(i)) {
                for f in own.chunks_exact(2) {
                    slot.add(f[0], f[1], 1.0);
                }
                slot.reduce(lowest);
            }
            send(false, own, after, gains);
            if node.out_on {
                let dst = match node.out.map(usize::from) {
                    None if stop_at.is_none() => Some(&mut *out),
                    Some(o) if o > i && o < count && play[o] => {
                        let at = (o - i - 1) * stride;
                        Some(&mut after[at..at + len])
                    }
                    _ => None,
                };
                if let Some(dst) = dst {
                    for (d, v) in dst.iter_mut().zip(own.iter()) {
                        *d += v;
                    }
                }
            }
        }
        if stop_at.is_some() {
            return missed;
        }

        let live = self
            .master
            .live
            .and_then(|l| self.live.fader(usize::from(l)));
        fill_gains(&self.master, live, master_glide, owns, start, gains);
        for (v, g) in out.iter_mut().zip(gains.iter()) {
            *v *= g;
        }
        let ceiling = self.ceiling;
        let (lowest, clamped) = if *limit {
            (limiter.process(ceiling, out), false)
        } else {
            let mut clamped = false;
            for v in out.iter_mut() {
                clamped |= v.abs() > ceiling;
                *v = v.clamp(-ceiling, ceiling);
            }
            (1.0, clamped)
        };
        if let Some(slot) = acc.and_then(|a| a.get_mut(meter::MASTER)) {
            for f in out.chunks_exact(2) {
                slot.add(f[0], f[1], f32::INFINITY);
            }
            slot.reduce(lowest);
            if clamped || lowest < LAMP_GAIN {
                slot.clip();
            }
        }
        missed
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
