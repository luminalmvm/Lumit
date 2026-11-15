//! Node racks that cannot stream: a rack holding an effect that has to hear
//! all of its input before its first block (a noise floor learned over the
//! whole recording) is baked whole when the plan is built, and its node
//! plays the baked sound as a clip (docs/09-AUDIO.md).
//!
//! Runs on the thread building the plan. The node's input is mixed through
//! the plan itself, so the rack hears what it would have heard live, then
//! baked as a clip's rack is, and kept by what it heard
//! ([`super::live::RackBakes`]).

use lumit_audio::block::{MixWork, SCRATCH_FRAMES};
use lumit_audio::plan::{MixPlan, PlacedClip, NONE};
use lumit_audio::rack::RackPuller;
use lumit_audio::stream::Source;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::bake::chain_bake_until;
use super::graph::Cancelled;
use super::jobs::AudioChain;
use super::live::{Baked, RackBakes, WholeKey};

/// How many frames of a node's input are mixed between looks at the cancel
/// flag.
const STEP_FRAMES: usize = 64 * SCRATCH_FRAMES;

/// Bake every node rack in `whole` (node index and its chain, in node order)
/// over the node's whole input, and play the result on the node in place of
/// its clips and its inputs. The rack's own [`lumit_audio::plan::PlanRack`]
/// is in `plan` until then, so mixing its input mixes only what feeds it.
pub(super) fn bake_all(
    mut plan: MixPlan,
    whole: Vec<(usize, Arc<AudioChain>)>,
    rate: u32,
    offline: bool,
    bakes: &mut RackBakes,
    cancel: &AtomicBool,
) -> Result<MixPlan, Cancelled> {
    let mut kept: Vec<(WholeKey, Baked)> = Vec::new();
    for (node, chain) in whole {
        match bake_one(&plan, node, chain, rate, offline, bakes, cancel) {
            Ok((key, baked)) => {
                plan = replace(plan, node, &baked, key.start_frame);
                kept.push((key, baked));
            }
            Err(Cancelled) => {
                bakes.whole.extend(kept);
                return Err(Cancelled);
            }
        }
    }
    bakes.whole = kept;
    Ok(plan)
}

/// Node `node`'s whole input, mixed through `plan`, and the rack's bake of it
/// (from the bakes when it heard the same before).
fn bake_one(
    plan: &MixPlan,
    node: usize,
    chain: Arc<AudioChain>,
    rate: u32,
    offline: bool,
    bakes: &mut RackBakes,
    cancel: &AtomicBool,
) -> Result<(WholeKey, Baked), Cancelled> {
    let stopped = || cancel.load(Ordering::Relaxed);
    let feeds = feeders(plan, node);
    let span = plan
        .clips
        .iter()
        .filter(|c| !c.silent && c.node != NONE)
        .filter(|c| feeds.get(usize::from(c.node)).copied().unwrap_or(false))
        .map(|c| (c.start_frame, c.start_frame + c.len as i64))
        .fold(None, |acc: Option<(i64, i64)>, (a, b)| {
            Some(acc.map_or((a, b), |(lo, hi)| (lo.min(a), hi.max(b))))
        });
    let Some((lo, hi)) = span else {
        let key = WholeKey {
            chain,
            rate,
            start_frame: 0,
            heard: [0; 32],
        };
        return Ok((key, None));
    };
    // From the first frame of sound, to a second past the last, so a tail
    // upstream is heard too.
    let lo = lo.max(0);
    let hi = hi.saturating_add(i64::from(rate)).max(lo);
    let frames = (hi - lo) as usize;
    let mut input = vec![0.0f32; frames * 2];
    let mut work = MixWork::new(rate);
    let mut racks = RackPuller::new(rate, offline);
    let mut at = lo as usize;
    for chunk in input.chunks_mut(STEP_FRAMES * 2) {
        if stopped() {
            return Err(Cancelled);
        }
        let n = chunk.len() / 2;
        plan.warm(at, at + n + rate as usize);
        plan.mix_block(at, chunk, &mut work, &mut racks, None, Some(node as u16));
        at += n;
    }
    let heard = fingerprint(&input);
    let key = WholeKey {
        chain,
        rate,
        start_frame: lo,
        heard,
    };
    if let Some(at) = bakes.whole.iter().position(|(k, _)| *k == key) {
        let (key, baked) = bakes.whole.swap_remove(at);
        return Ok((key, baked));
    }
    let baked = chain_bake_until(&key.chain, &input, lo, rate, offline, cancel);
    if stopped() {
        return Err(Cancelled);
    }
    let (samples, latency) = baked.unwrap_or((input, 0));
    let source = Source::whole(Arc::new(lumit_media::audio::AudioBuffer { rate, samples }));
    Ok((key, Some((source, latency))))
}

/// `plan` with node `node` playing `baked`, whose input started at
/// `start_frame` (placed its latency early, so the wet lands where the dry
/// did), in place of its own clips, its inputs and its rack.
fn replace(mut plan: MixPlan, node: usize, baked: &Baked, start_frame: i64) -> MixPlan {
    let n = node as u16;
    plan.clips.retain(|c| c.node != n);
    for other in &mut plan.nodes {
        if other.out == Some(n) {
            other.out_on = false;
        }
        for send in other.sends.iter_mut().filter(|s| s.to == n) {
            send.on = false;
        }
    }
    if let Some(r) = plan.nodes.get_mut(node).and_then(|x| x.rack.take()) {
        if usize::from(r) < plan.racks.len() {
            plan.racks.remove(usize::from(r));
        }
        for other in &mut plan.nodes {
            if let Some(o) = other.rack.as_mut().filter(|o| **o > r) {
                *o -= 1;
            }
        }
    }
    if let Some((source, latency)) = baked {
        let len = source.frames().unwrap_or(0);
        plan.clips.push(PlacedClip {
            source: Arc::clone(source),
            start_frame: start_frame - i64::from(*latency),
            src_start: 0,
            len,
            gain: [1.0, 1.0],
            envelope: None,
            node: n,
            silent: false,
        });
    }
    let total = plan.total_frames;
    let fallback = plan.clips.clone();
    plan.finish()
        .unwrap_or_else(|_| super::graph::unrouted(fallback, total))
}

/// Which nodes' sound reaches node `n`'s input: those feeding it, those
/// feeding them, and the keys of their ducks; `n` itself too.
fn feeders(plan: &MixPlan, n: usize) -> Vec<bool> {
    let feeds = |m: usize, to: usize| {
        plan.nodes.get(m).is_some_and(|from| {
            from.out.map(usize::from) == Some(to)
                || from.sends.iter().any(|s| usize::from(s.to) == to)
        })
    };
    let mut needed = vec![false; plan.nodes.len().max(n + 1)];
    needed[n] = true;
    for m in (0..n).rev() {
        needed[m] = (m + 1..=n).any(|k| {
            needed[k]
                && (feeds(m, k)
                    || plan
                        .nodes
                        .get(k)
                        .and_then(|x| x.duck)
                        .is_some_and(|d| usize::from(d.key) == m))
        });
    }
    needed
}

/// A fingerprint of a node's input, to tell whether the rack heard the same
/// sound last time.
fn fingerprint(samples: &[f32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    let mut bytes = Vec::with_capacity(samples.len().min(1 << 14) * 4);
    for chunk in samples.chunks(1 << 14) {
        bytes.clear();
        for s in chunk {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        h.update(&bytes);
    }
    *h.finalize().as_bytes()
}
