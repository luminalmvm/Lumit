//! A comp's sound as a plan the mix is made from: the placed clips, the
//! nodes they sum into, and how the nodes feed each other and the master
//! (docs/09-AUDIO.md).
//!
//! # In plain terms
//!
//! A mixing desk has a strip per layer and a strip per bus. Each layer's
//! clips sum into its strip; a strip's fader sets its level and its output
//! goes to a bus or the master, and its sends feed buses as well. This is
//! that desk written down: clips name the **node** they sum into, nodes are
//! listed so that every edge runs to a later one, and the master sits after
//! them all. [`MixPlan::mix_block`] walks it once a buffer.
//!
//! A plan is built on the bridge's thread, finished there ([`MixPlan::finish`]
//! checks it and works out what each rack needs), and shared read-only with
//! the audio callback, the filling thread and the rack thread. Editing sound
//! (solo, mute, a move, a trim) is building a new one and swapping it in. The
//! readers here ([`MixPlan::peaks`], [`MixPlan::frame_at`] and the rest) run
//! on whichever thread asks and may wait for a decode; none of them is for
//! the callback.

use crate::duck::Duck;
use crate::live::LiveBank;
use crate::mix::{GainEnvelope, MASTER_CEILING};
use crate::rack::{RackOpener, RackRing};
use crate::stream::Source;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The most nodes a plan may have. Every mix keeps a buffer per node, made
/// before the stream opens, so this is fixed. A builder with more strips than
/// this folds the rest into their clips and sends them straight to the master.
pub const MAX_NODES: usize = 256;

/// The most sends whose release can glide back to the keys. Past this many a
/// released send goes back at once.
pub const MAX_SENDS: usize = 1024;

/// "No index": a clip with no node (straight to the master), a node that is
/// new in [`MixPlan::carry`].
pub const NONE: u16 = u16::MAX;

/// How long a block a waiting caller has just had decoded stays safe from
/// being dropped for another, in milliseconds: long enough to be read.
const WARM_HOLD_MS: u64 = 2_000;

/// How many frames of a bucket [`MixPlan::peaks`] reads at most before it
/// starts striding: enough that a column's level is honest, few enough that
/// four thousand columns over a long comp stay a short walk.
pub const PEAK_SAMPLES_PER_BUCKET: usize = 256;

/// One placed clip in a live [`MixPlan`]: a shared source, where it lands on
/// the comp strip, which stretch of it plays, its gain and the node it sums
/// into. The same placement triple [`crate::mix::place_on_timeline`]
/// produces.
#[derive(Clone)]
pub struct PlacedClip {
    /// The sound: a file read in blocks, or a buffer already in memory
    /// ([`crate::stream`]).
    pub source: Arc<Source>,
    /// Output frame where the source's frame `src_start` lands (may be
    /// negative).
    pub start_frame: i64,
    /// The first frame of the source that plays.
    pub src_start: usize,
    /// How many frames of it play.
    pub len: usize,
    /// `[left, right]` linear gain: the clip's own gain and everything folded
    /// into it below its strip. Used when `envelope` is None; an animated one
    /// rides the envelope instead.
    pub gain: [f32; 2],
    /// Control-rate gain curve, indexed on placed frames (0 = this clip's
    /// first audible frame). Shared so plan clones stay cheap.
    pub envelope: Option<Arc<GainEnvelope>>,
    /// The node this clip sums into, or [`NONE`] for straight to the master
    /// with no meter.
    pub node: u16,
    /// Its node has no path that is heard (muted, or silenced by a solo), so
    /// the mix and the filling thread pass it by.
    pub silent: bool,
}

/// A fader: a node's Volume and Pan, the master, or a send's level.
#[derive(Clone, Debug, PartialEq)]
pub struct Fader {
    /// `[left, right]` linear gain, used when `envelope` is None.
    pub gain: [f32; 2],
    /// Its keys as a control-rate curve, indexed on timeline frames from 0.
    pub envelope: Option<Arc<GainEnvelope>>,
    /// Its override in the plan's [`LiveBank`]: a fader index for a node or
    /// the master, a send index for a send. None for a fader nobody holds.
    pub live: Option<u16>,
}

impl Default for Fader {
    fn default() -> Self {
        Self::constant([1.0, 1.0])
    }
}

impl Fader {
    /// A fader that stays at `gain`.
    #[must_use]
    pub fn constant(gain: [f32; 2]) -> Self {
        Self {
            gain,
            envelope: None,
            live: None,
        }
    }

    /// The gain at timeline frame `frame`, from the keys or the constant. A
    /// held override is the mix's business, not this.
    #[inline]
    #[must_use]
    pub fn at(&self, frame: usize) -> [f32; 2] {
        self.envelope
            .as_ref()
            .map_or(self.gain, |e| e.gain_at(frame))
    }
}

/// A send from a node to a later one.
#[derive(Clone, Debug, PartialEq)]
pub struct MixSend {
    /// The node sent to.
    pub to: u16,
    /// The send's level, the same on both sides.
    pub level: Fader,
    /// Taken before the node's fader rather than after it.
    pub pre: bool,
    /// False for a send that is switched off: it adds nothing.
    pub on: bool,
}

/// One strip of the desk: a layer that makes sound, or a bus.
#[derive(Clone, Debug, PartialEq)]
pub struct MixNode {
    /// Volume and Pan.
    pub fader: Fader,
    /// The node fed after the fader, or None for the master.
    pub out: Option<u16>,
    /// False when the strip is muted or silenced by a solo: its output adds
    /// nothing (its sends still do).
    pub out_on: bool,
    /// Its sends, pre and post fader.
    pub sends: Vec<MixSend>,
    /// Its rack, an index into [`MixPlan::racks`].
    pub rack: Option<u16>,
    /// Lowered while another node plays.
    pub duck: Option<Duck>,
}

impl Default for MixNode {
    fn default() -> Self {
        Self {
            fader: Fader::default(),
            out: None,
            out_on: true,
            sends: Vec::new(),
            rack: None,
            duck: None,
        }
    }
}

/// A racked node: what opens its rack, and the ring the rack thread fills
/// for the callback.
#[derive(Clone)]
pub struct PlanRack {
    /// The node it runs on.
    pub node: u16,
    /// What opens the rack's stream.
    pub opener: Arc<dyn RackOpener>,
    /// Where its output waits for the callback. Handed from plan to plan
    /// while the rack stays the same, so an edit does not cut a tail.
    pub ring: Arc<RackRing>,
}

/// What [`MixPlan::finish`] refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("node {node} feeds or listens to a node that is not after it")]
    Order { node: usize },
    #[error("{nodes} nodes is more than a mix holds")]
    TooManyNodes { nodes: usize },
    #[error("clip {clip} sums into a node the plan does not have")]
    NoNode { clip: usize },
    #[error("rack {rack} and its node do not name each other")]
    Rack { rack: usize },
}

/// A comp's audio as a *plan* rather than a baked buffer. The realtime
/// callback mixes the clips sounding in each buffer through the nodes
/// ([`MixPlan::mix_block`]), so editing audio is a plan swap, heard on the
/// next callback. The clips' sound is not in the plan: each names a
/// [`Source`], whose blocks come and go under the pool's budget
/// ([`crate::stream`]), so a plan for a two-hour cut is a few hundred
/// kilobytes whatever it plays.
#[derive(Clone)]
pub struct MixPlan {
    /// Every clip, each summing into its node.
    pub clips: Vec<PlacedClip>,
    /// The desk's strips, each feeding only later ones.
    pub nodes: Vec<MixNode>,
    /// How long the mix is.
    pub total_frames: usize,
    /// The master fader, ahead of the limiter.
    pub master: Fader,
    /// The limiter's ceiling as a linear true peak.
    pub ceiling: f32,
    /// The racked nodes' racks.
    pub racks: Vec<PlanRack>,
    /// Overrides while a control is held.
    pub live: Arc<LiveBank>,
    /// Each node's index in the plan before this one, or [`NONE`], so a held
    /// fader, a duck and a glide carry over an edit.
    pub carry: Vec<u16>,
    /// Per rack, the nodes its input needs. Worked out by [`Self::finish`].
    pub(crate) ancestors: Vec<Vec<u16>>,
    /// Which plan this is, so a mix can tell a new one from the one it was
    /// mixing. Not the plan's address: a freed plan's address comes back.
    pub(crate) id: u64,
}

/// A number no plan has had before.
fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Default for MixPlan {
    fn default() -> Self {
        Self {
            clips: Vec::new(),
            nodes: Vec::new(),
            total_frames: 0,
            master: Fader::default(),
            ceiling: MASTER_CEILING,
            racks: Vec::new(),
            live: Arc::new(LiveBank::default()),
            carry: Vec::new(),
            ancestors: Vec::new(),
            id: next_id(),
        }
    }
}

impl MixPlan {
    /// The simplest desk: every clip into one node at unity, and that node to
    /// the master. A buffer played on its own, a preview.
    #[must_use]
    pub fn single(mut clips: Vec<PlacedClip>, total_frames: usize) -> Self {
        for clip in &mut clips {
            clip.node = 0;
        }
        Self {
            clips,
            nodes: vec![MixNode::default()],
            total_frames,
            ..Self::default()
        }
    }

    /// Check the plan and work out what the mix needs to know about it: every
    /// output, send and duck key runs to a later node (a duck's key to an
    /// earlier one), the plan holds no more than [`MAX_NODES`] nodes, every
    /// clip's node is there, and each rack and its node name each other.
    pub fn finish(mut self) -> Result<Self, PlanError> {
        let count = self.nodes.len();
        if count > MAX_NODES {
            return Err(PlanError::TooManyNodes { nodes: count });
        }
        for (c, clip) in self.clips.iter().enumerate() {
            if !clip.silent && clip.node != NONE && usize::from(clip.node) >= count {
                return Err(PlanError::NoNode { clip: c });
            }
        }
        for (i, node) in self.nodes.iter().enumerate() {
            let later = |to: u16| usize::from(to) > i && usize::from(to) < count;
            let ordered = node.out.is_none_or(later)
                && node.sends.iter().all(|s| later(s.to))
                && node.duck.is_none_or(|d| usize::from(d.key) < i);
            if !ordered {
                return Err(PlanError::Order { node: i });
            }
            if let Some(r) = node.rack {
                let named = self
                    .racks
                    .get(usize::from(r))
                    .is_some_and(|rack| usize::from(rack.node) == i);
                if !named {
                    return Err(PlanError::Rack {
                        rack: usize::from(r),
                    });
                }
            }
        }
        for (r, rack) in self.racks.iter().enumerate() {
            let named = self
                .nodes
                .get(usize::from(rack.node))
                .is_some_and(|n| n.rack.map(usize::from) == Some(r));
            if !named {
                return Err(PlanError::Rack { rack: r });
            }
        }
        self.ancestors = self
            .racks
            .iter()
            .map(|rack| self.ancestors_of(usize::from(rack.node)))
            .collect();
        self.id = next_id();
        Ok(self)
    }

    /// The nodes whose sound reaches node `n`'s input: those that feed it,
    /// those that feed them, and the keys of their ducks.
    fn ancestors_of(&self, n: usize) -> Vec<u16> {
        let feeds = |from: &MixNode, to: usize| {
            from.out.map(usize::from) == Some(to)
                || from.sends.iter().any(|s| usize::from(s.to) == to)
        };
        let mut needed = vec![false; n];
        // Edges run upward, so walking down decides each node after every
        // node it could feed.
        for m in (0..n).rev() {
            let from = &self.nodes[m];
            needed[m] = feeds(from, n)
                || (m + 1..n).any(|k| {
                    needed[k]
                        && (feeds(from, k)
                            || self.nodes[k].duck.is_some_and(|d| usize::from(d.key) == m))
                });
        }
        (0..n).filter(|&m| needed[m]).map(|m| m as u16).collect()
    }

    /// Seed this plan's [`LiveBank`] from `old`'s, through [`Self::carry`]: a
    /// fader or a send held or gliding in the old plan is held or gliding in
    /// this one. A send carries when its node and the node it goes to both
    /// carry.
    pub fn seed_live(&self, old: &MixPlan) {
        let was = |n: usize| {
            self.carry
                .get(n)
                .filter(|&&o| o != NONE)
                .and_then(|&o| old.nodes.get(usize::from(o)).map(|node| (o, node)))
        };
        let faders = self.nodes.len();
        let mut fader_carry = vec![NONE; faders];
        let mut send_carry = vec![NONE; MAX_SENDS];
        for (n, node) in self.nodes.iter().enumerate() {
            let Some((_, old_node)) = was(n) else {
                continue;
            };
            if let (Some(slot), Some(from)) = (node.fader.live, old_node.fader.live) {
                if let Some(c) = fader_carry.get_mut(usize::from(slot)) {
                    *c = from;
                }
            }
            for send in &node.sends {
                let to = was(usize::from(send.to)).map(|(o, _)| o);
                let from = old_node
                    .sends
                    .iter()
                    .find(|s| Some(s.to) == to)
                    .and_then(|s| s.level.live);
                if let (Some(slot), Some(from)) = (send.level.live, from) {
                    if let Some(c) = send_carry.get_mut(usize::from(slot)) {
                        *c = from;
                    }
                }
            }
        }
        self.live.seed_from(&old.live, &fader_carry, &send_carry);
    }

    /// The mix summarised over `[start_s, end_s)` in `buckets` buckets, as the
    /// waveform lanes take a source's peaks: `min`, `max`, `rms` per bucket,
    /// each in −1..1, laid out as `3 * bucket`. What the Timeline's Sound mix
    /// row draws, read through [`Self::frame_at`].
    ///
    /// A bucket longer than [`PEAK_SAMPLES_PER_BUCKET`] frames is read on a
    /// stride, so a comp-wide view costs a bounded walk whatever the comp's
    /// length.
    // ponytail: a strided bucket can miss a one-frame transient at a wide
    // zoom; a pyramid built on the prepare worker is the upgrade if the
    // row ever needs to be cut against rather than read.
    #[must_use]
    pub fn peaks(&self, rate: u32, start_s: f64, end_s: f64, buckets: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; buckets * 3];
        if buckets == 0 || rate == 0 || end_s <= start_s {
            return out;
        }
        let rate_f = f64::from(rate);
        let step_s = (end_s - start_s) / buckets as f64;
        for b in 0..buckets {
            let from = ((start_s + step_s * b as f64) * rate_f).floor().max(0.0) as usize;
            let to = ((start_s + step_s * (b + 1) as f64) * rate_f)
                .ceil()
                .max(0.0) as usize;
            let to = to.min(self.total_frames);
            if to <= from {
                continue;
            }
            let stride = ((to - from) / PEAK_SAMPLES_PER_BUCKET).max(1);
            let (mut lo, mut hi, mut sq, mut n) = (0.0f32, 0.0f32, 0.0f64, 0u32);
            // The clips sounding in this bucket, found once for the bucket
            // and not once per frame read from it.
            let sounding: Vec<&PlacedClip> = self
                .clips
                .iter()
                .filter(|c| Self::sounding(c, from, to).is_some())
                .collect();
            let mut i = from;
            while i < to {
                let (l, r) = self.frame_of(i, sounding.iter().copied());
                lo = lo.min(l.min(r));
                hi = hi.max(l.max(r));
                sq += f64::from(l * l + r * r) * 0.5;
                n += 1;
                i += stride;
            }
            if n > 0 {
                out[b * 3] = lo;
                out[b * 3 + 1] = hi;
                out[b * 3 + 2] = (sq / f64::from(n)).sqrt() as f32;
            }
        }
        out
    }

    /// [`Self::peaks`] for a window too wide to read sample by sample: every
    /// file clip's part of each bucket comes from that file's own peak
    /// pyramid, asked of `pyramid_of`, and the parts are put together.
    ///
    /// # In plain terms
    ///
    /// Two hours of mix across a row two thousand columns wide is three and a
    /// half seconds a column. Reading that off the samples would mean having
    /// every file of the cut decoded, which is the thing a long cut cannot
    /// do. Each file already has a summary of itself at every zoom, so the
    /// row asks those: a clip's swing in a column is its file's swing over
    /// the same stretch, scaled by the clip's gain and the faders on its way
    /// out to the master.
    ///
    /// Where one clip sounds in a column, which is most columns of a cut,
    /// that is the mix. Where two overlap it is an upper bound on the swing
    /// and the energies are added as if the two were unrelated sound. A
    /// picture of how loud the mix is, not a measurement of it.
    ///
    /// A clip with no pyramid (a baked rack's output, a file that will not
    /// summarise) is read from whatever of it is in memory.
    #[must_use]
    pub fn peaks_wide(
        &self,
        rate: u32,
        start_s: f64,
        end_s: f64,
        buckets: usize,
        pyramid_of: &mut dyn FnMut(
            &lumit_media::audio::SoundSource,
        ) -> Option<Arc<crate::peaks::PeakPyramid>>,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; buckets * 3];
        if buckets == 0 || rate == 0 || end_s <= start_s {
            return out;
        }
        let rate_f = f64::from(rate);
        let step_s = (end_s - start_s) / buckets as f64;
        // Each bucket's low, high and mean square, summed over its clips.
        let mut sums = vec![(0.0f32, 0.0f32, 0.0f64); buckets];
        for clip in self.clips.iter().filter(|c| !c.silent) {
            let c0 = clip.start_frame as f64 / rate_f;
            let c1 = c0 + clip.len as f64 / rate_f;
            let (from_s, to_s) = (
                c0.max(start_s),
                c1.min(end_s).min(self.total_frames as f64 / rate_f),
            );
            if to_s <= from_s {
                continue;
            }
            let pyramid = clip.source.sound().and_then(&mut *pyramid_of);
            let first = ((from_s - start_s) / step_s).floor().max(0.0) as usize;
            let last = (((to_s - start_s) / step_s).ceil().max(0.0) as usize).min(buckets);
            for (b, sum) in sums.iter_mut().enumerate().take(last).skip(first) {
                let b0 = start_s + step_s * b as f64;
                let (a, z) = (b0.max(from_s), (b0 + step_s).min(to_s));
                if z <= a {
                    continue;
                }
                // The clip's frames under this bucket, and its gain half way.
                let at = ((a - c0) * rate_f) as usize;
                let count = (((z - a) * rate_f) as usize).max(1);
                let own = clip
                    .envelope
                    .as_ref()
                    .map_or(clip.gain, |e| e.gain_at(at + count / 2));
                let path = self.path_gain(clip.node, (a * rate_f) as usize + count / 2);
                let g = [own[0] * path[0], own[1] * path[1]];
                let (lo, hi, rms) = match &pyramid {
                    Some(pyramid) => {
                        let s0 = (clip.src_start + at) as f64 / rate_f;
                        let w = pyramid.window(crate::peaks::Band::Full, s0, s0 + (z - a));
                        // The pyramid is of the two sides' mean.
                        let gm = 0.5 * (g[0] + g[1]);
                        (w.min * gm, w.max * gm, w.rms * gm)
                    }
                    None => {
                        let stride = (count / PEAK_SAMPLES_PER_BUCKET).max(1);
                        let (mut lo, mut hi, mut sq, mut n) = (0.0f32, 0.0f32, 0.0f64, 0u32);
                        for i in (at..at + count).step_by(stride) {
                            let Some((l, r)) = clip.source.frame(clip.src_start + i) else {
                                continue;
                            };
                            let (l, r) = (l * g[0], r * g[1]);
                            lo = lo.min(l.min(r));
                            hi = hi.max(l.max(r));
                            sq += f64::from(l * l + r * r) * 0.5;
                            n += 1;
                        }
                        let rms = if n == 0 {
                            0.0
                        } else {
                            (sq / f64::from(n)).sqrt() as f32
                        };
                        (lo, hi, rms)
                    }
                };
                sum.0 += lo;
                sum.1 += hi;
                // Weighted by how much of the bucket the clip covers.
                sum.2 += f64::from(rms * rms) * ((z - a) / step_s);
            }
        }
        let ceiling = self.ceiling;
        for (b, (lo, hi, sq)) in sums.into_iter().enumerate() {
            let at = ((start_s + step_s * (b as f64 + 0.5)) * rate_f).max(0.0) as usize;
            let g = self.master.at(at);
            let gm = 0.5 * (g[0] + g[1]);
            let held = |v: f32| (v * gm).clamp(-ceiling, ceiling);
            out[b * 3] = held(lo);
            out[b * 3 + 1] = held(hi);
            out[b * 3 + 2] = held(sq.sqrt() as f32);
        }
        out
    }

    /// The faders a node's sound passes on its way out to the master, at
    /// timeline frame `frame`, multiplied together; the master's own is not
    /// in it. Sends are not followed, and a strip that is off gives nought.
    fn path_gain(&self, node: u16, frame: usize) -> [f32; 2] {
        let mut g = [1.0f32, 1.0];
        let mut at = node;
        // At most one step per node: edges only run upward.
        for _ in 0..=self.nodes.len() {
            if at == NONE {
                return g;
            }
            let Some(n) = self.nodes.get(usize::from(at)) else {
                return g;
            };
            if !n.out_on {
                return [0.0, 0.0];
            }
            let f = n.fader.at(frame);
            g = [g[0] * f[0], g[1] * f[1]];
            at = n.out.unwrap_or(NONE);
        }
        g
    }

    /// The `(left, right)` of output frame `i` through the whole desk: every
    /// covering clip into its node, each node through its sends, fader and
    /// output, then the master fader and the ceiling as a plain clamp. Racks
    /// are passed through dry, ducks and held faders left out, since each
    /// needs the sound before it. A frame whose block is not in memory reads
    /// as silence.
    ///
    /// What [`Self::mix_block`] is held against with its racks dry and its
    /// limiter off, and what the readers that want a frame here and there
    /// use. Not for the callback.
    #[must_use]
    pub fn frame_at(&self, i: usize) -> (f32, f32) {
        self.frame_of(i, self.clips.iter())
    }

    /// [`Self::frame_at`] over the clips the caller has already narrowed to.
    fn frame_of<'a>(&self, i: usize, clips: impl Iterator<Item = &'a PlacedClip>) -> (f32, f32) {
        let count = self.nodes.len().min(MAX_NODES);
        let mut acc = [[0.0f32; 2]; MAX_NODES];
        let mut master = [0.0f32; 2];
        for clip in clips.filter(|c| !c.silent) {
            let Ok(idx) = usize::try_from(i as i64 - clip.start_frame) else {
                continue; // this clip starts later
            };
            if idx >= clip.len {
                continue; // this clip has ended
            }
            let dst = if clip.node == NONE {
                &mut master
            } else {
                match acc
                    .get_mut(usize::from(clip.node))
                    .filter(|_| clip.node < count as u16)
                {
                    Some(dst) => dst,
                    None => continue,
                }
            };
            if let Some((sl, sr)) = clip.source.frame(clip.src_start + idx) {
                let g = clip.envelope.as_ref().map_or(clip.gain, |e| e.gain_at(idx));
                dst[0] += sl * g[0];
                dst[1] += sr * g[1];
            }
        }
        for (n, node) in self.nodes.iter().enumerate().take(count) {
            let mut v = acc[n];
            let send = |acc: &mut [[f32; 2]; MAX_NODES], v: [f32; 2], pre: bool| {
                for s in node.sends.iter().filter(|s| s.pre == pre && s.on) {
                    let to = usize::from(s.to);
                    if to > n && to < count {
                        let g = s.level.at(i);
                        acc[to][0] += v[0] * g[0];
                        acc[to][1] += v[1] * g[1];
                    }
                }
            };
            send(&mut acc, v, true);
            let g = node.fader.at(i);
            v = [v[0] * g[0], v[1] * g[1]];
            send(&mut acc, v, false);
            if node.out_on {
                match node.out.map(usize::from) {
                    None => {
                        master[0] += v[0];
                        master[1] += v[1];
                    }
                    Some(o) if o > n && o < count => {
                        acc[o][0] += v[0];
                        acc[o][1] += v[1];
                    }
                    Some(_) => {}
                }
            }
        }
        let g = self.master.at(i);
        let c = self.ceiling;
        (
            (master[0] * g[0]).clamp(-c, c),
            (master[1] * g[1]).clamp(-c, c),
        )
    }

    /// The stretch of `clip` sounding in output frames `[from, to)`: how far
    /// into the stretch asked for it begins, the placed frame it begins at,
    /// and how many frames.
    pub(crate) fn sounding(
        clip: &PlacedClip,
        from: usize,
        to: usize,
    ) -> Option<(usize, usize, usize)> {
        let a = (from as i64).max(clip.start_frame);
        let b = (to as i64).min(clip.start_frame.saturating_add(clip.len as i64));
        (b > a).then(|| {
            (
                (a - from as i64) as usize,
                (a - clip.start_frame) as usize,
                (b - a) as usize,
            )
        })
    }

    /// **One step of filling ahead.** Mark every block sounding in output
    /// frames `[from, to)` as wanted until `until` (on
    /// [`crate::stream::now`]'s clock) and decode the one that is due soonest
    /// and not in memory. `false` when there was nothing to decode: the
    /// stretch is ready to play. A silent clip is not heard, so it is not
    /// filled.
    ///
    /// One block a call, so the thread that fills ahead looks at the playhead
    /// again between blocks and a seek is answered by the next decode, not
    /// after a queue of them. Waits for the decode; never called from the
    /// callback.
    pub fn fill_step(&self, from: usize, to: usize, until: u64) -> bool {
        let mut next: Option<(usize, &Arc<Source>, usize)> = None;
        for clip in self.clips.iter().filter(|c| !c.silent) {
            let Some((at, first, count)) = Self::sounding(clip, from, to) else {
                continue;
            };
            if let Some((block, into)) = clip.source.want(clip.src_start + first, count, until) {
                if next.as_ref().is_none_or(|n| at + into < n.0) {
                    next = Some((at + into, &clip.source, block));
                }
            }
        }
        match next {
            Some((_, source, block)) => {
                let _ = source.block(block, false, until);
                true
            }
            None => false,
        }
    }

    /// Whether `[from, from + frames)` can be mixed with nothing missing:
    /// every clip's sound there in memory and every rack's ring holding it.
    /// Never waits; what a start looks at before it plays.
    #[must_use]
    pub fn ready(&self, from: usize, frames: usize) -> bool {
        let to = from.saturating_add(frames);
        let clips = self.clips.iter().filter(|c| !c.silent).all(|clip| {
            Self::sounding(clip, from, to)
                .is_none_or(|(_, first, count)| clip.source.ready(clip.src_start + first, count))
        });
        let (at, end) = (from as i64, to as i64);
        clips
            && self
                .racks
                .iter()
                .all(|r| r.ring.valid_from() <= at && r.ring.produced_to() >= end)
    }

    /// Decode everything sounding in `[from, to)` that is not in memory, and
    /// wait for it: a scrub about to sound, a stretch about to be summarised.
    pub fn warm(&self, from: usize, to: usize) {
        let until = crate::stream::now().saturating_add(WARM_HOLD_MS);
        while self.fill_step(from, to, until) {}
    }

    /// How many frames of file sound the plan places, every clip counted: what
    /// the whole plan would hold if all of it were in memory at once.
    #[must_use]
    pub fn file_frames(&self) -> usize {
        self.clips
            .iter()
            .filter(|c| c.source.is_file())
            .map(|c| c.len)
            .sum()
    }

    /// Timeline waveform peaks straight off the plan, no whole-comp buffer
    /// ever made. Same bucket shape as [`crate::mix::waveform_peaks`].
    #[must_use]
    pub fn waveform_peaks(&self, buckets: usize) -> Vec<(f32, f32)> {
        if self.total_frames == 0 || buckets == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(buckets);
        for b in 0..buckets {
            let start = b * self.total_frames / buckets;
            let end =
                (((b + 1) * self.total_frames / buckets).max(start + 1)).min(self.total_frames);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in start..end {
                let (l, r) = self.frame_at(i);
                let m = 0.5 * (l + r);
                lo = lo.min(m);
                hi = hi.max(m);
            }
            if lo > hi {
                (lo, hi) = (0.0, 0.0);
            }
            out.push((lo, hi));
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::mix::{mix_stereo, PlacedAudio};

    fn tone(frames: usize, value: f32) -> Vec<f32> {
        vec![value; frames * 2]
    }

    fn whole(samples: Vec<f32>) -> Arc<Source> {
        Source::whole(Arc::new(lumit_media::audio::AudioBuffer {
            rate: 48_000,
            samples,
        }))
    }

    fn clip(source: Arc<Source>, start_frame: i64, src_start: usize, len: usize) -> PlacedClip {
        PlacedClip {
            source,
            start_frame,
            src_start,
            len,
            gain: [1.0, 1.0],
            envelope: None,
            node: 0,
            silent: false,
        }
    }

    /// A volume fade must sound identical through the baked mixer and the
    /// live plan: the same preview == export contract the static mix keeps.
    #[test]
    fn an_enveloped_fade_rides_through_both_mixers_identically() {
        let env = GainEnvelope {
            stride: 2,
            points: vec![[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]],
        };
        let src = tone(4, 0.8);
        let baked = mix_stereo(
            &[PlacedAudio {
                start_frame: 0,
                samples: &src,
                gain: [1.0, 1.0],
                envelope: Some(env.clone()),
            }],
            4,
        );
        // Placed frames 0..4 fade 0 → 1: gains 0, 0.25, 0.5, 0.75.
        for (i, want) in [0.0f32, 0.2, 0.4, 0.6].iter().enumerate() {
            assert!(
                (baked[i * 2] - want).abs() < 1e-6,
                "frame {i}: {} vs {want}",
                baked[i * 2]
            );
        }
        let plan = MixPlan::single(
            vec![PlacedClip {
                envelope: Some(Arc::new(env)),
                ..clip(whole(src), 0, 0, 4)
            }],
            4,
        );
        for i in 0..4 {
            let (l, r) = plan.frame_at(i);
            assert!(
                (l - baked[i * 2]).abs() < 1e-6 && (r - baked[i * 2 + 1]).abs() < 1e-6,
                "frame {i}: plan and baked mixes disagree"
            );
        }
    }

    /// The Sound mix row's peaks are the mix itself, bucketed: a half-scale
    /// tone over the first half of a plan reads 0.5 in the first bucket and
    /// silence in the second, and a bucket long enough to be strided still
    /// finds the level it holds.
    #[test]
    fn plan_peaks_bucket_the_mixed_output() {
        let rate = 100;
        let mut samples = vec![0.5f32; 100];
        samples.extend(std::iter::repeat_n(0.0f32, 100));
        let plan = MixPlan::single(vec![clip(whole(samples), 0, 0, 100)], 100);
        let peaks = plan.peaks(rate, 0.0, 1.0, 2);
        assert_eq!(peaks.len(), 6);
        assert!(
            (peaks[1] - 0.5).abs() < 1e-6,
            "the first bucket's max is the tone"
        );
        assert!((peaks[2] - 0.5).abs() < 1e-6, "and so is its rms");
        assert_eq!(&peaks[3..], &[0.0, 0.0, 0.0], "the second bucket is silent");

        // A plan thirty thousand frames long is strided, and the tone still
        // reads through the stride.
        let wide = MixPlan::single(
            vec![clip(whole(vec![0.5f32; 60_000]), 0, 0, 30_000)],
            30_000,
        );
        let peaks = wide.peaks(rate, 0.0, 300.0, 1);
        assert!((peaks[1] - 0.5).abs() < 1e-6);

        // Nothing to bucket comes back as silence, never a panic.
        assert!(plan.peaks(rate, 1.0, 0.5, 4).iter().all(|v| *v == 0.0));
        assert!(plan.peaks(rate, 0.0, 1.0, 0).is_empty());
    }

    /// The live plan must sound exactly like the baked mix: same placements,
    /// same overlap summing, same ceiling, sample for sample.
    #[test]
    fn a_mix_plan_matches_the_baked_mix_sample_for_sample() {
        let (a, b) = (tone(6, 0.6), tone(4, 0.7));
        // a: frames 0..6 at 0.6; b: frames 4..8 at 0.7, so the overlap sums
        // and clamps to the master ceiling; head and tail come from one each.
        let baked = mix_stereo(
            &[
                PlacedAudio {
                    start_frame: 0,
                    samples: &a,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
                PlacedAudio {
                    start_frame: 4,
                    samples: &b,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
            ],
            10,
        );
        let plan = MixPlan::single(vec![clip(whole(a), 0, 0, 6), clip(whole(b), 4, 0, 4)], 10);
        for i in 0..10 {
            let (l, r) = plan.frame_at(i);
            assert!(
                (l - baked[i * 2]).abs() < 1e-6 && (r - baked[i * 2 + 1]).abs() < 1e-6,
                "frame {i}: plan ({l},{r}) vs baked ({},{})",
                baked[i * 2],
                baked[i * 2 + 1]
            );
        }
        // Trimmed clips read the right slice: src_start offsets into the source.
        let c = whole(
            (0..8)
                .flat_map(|n| [n as f32 * 0.1, n as f32 * 0.1])
                .collect(),
        );
        let trimmed = MixPlan::single(vec![clip(c, 0, 3, 2)], 4);
        assert!((trimmed.frame_at(0).0 - 0.3).abs() < 1e-6);
        assert!((trimmed.frame_at(1).0 - 0.4).abs() < 1e-6);
        assert_eq!(trimmed.frame_at(2), (0.0, 0.0), "past the trim: silence");
        // And the waveform straight off the plan matches the buckets' extremes.
        let peaks = trimmed.waveform_peaks(2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0].1 - 0.4).abs() < 1e-6);
    }
}
