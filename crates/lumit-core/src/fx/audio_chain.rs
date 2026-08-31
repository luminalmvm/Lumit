//! The layer's **audio insert chain**: sound in, sound out, one plugin at a
//! time (docs/impl/audio-plugins.md §2 and §3, K-700).
//!
//! # In plain terms
//!
//! An audio plugin is an entry in the layer's ordinary effect stack — "the
//! stack is the rack", there is no separate FX list. The *chain* is the
//! audio-typed subset of that stack, in stack order, and it sits **ahead of
//! Volume and Pan**: a fade must fade the processed sound, or riding the fader
//! down through a compressor would change the compression amount mid-fade and
//! audibly pump.
//!
//! This module is the part of that with no opinion about CLAP, VST3, brokers or
//! documents: given a run of interleaved stereo samples and a list of live
//! processors, it hands back the processed run. Everything it knows is
//! arithmetic and one trait call per block, so every rule here is a plain
//! deterministic test.
//!
//! # The three rules worth reading before changing anything
//!
//! **Blocks are a fact about the layer, not about the playhead.** A chain
//! always processes from its own first sample in [`AUDIO_BLOCK_FRAMES`]-frame
//! blocks, so two runs of the same input produce identical output and the
//! export is the preview's own arithmetic rather than a second opinion
//! (docs/09 §8).
//!
//! **A dying plugin costs one block.** [`AudioProcessor::process`] answering
//! `false` — a crash, a missed deadline, a plugin the user switched off — ships
//! that block **dry**: the chain's input, unchanged. In a montage the music
//! continuing slightly wrong beats a hole in it.
//!
//! **A splice gets a ramp.** A dry block spliced straight into wet sound clicks,
//! which is worse than either. So each edge of a dry stretch is crossfaded over
//! [`AUDIO_SPLICE_FRAMES`] on the wet side of the boundary — by the time the dry
//! block starts the signal is already dry, and it is already wet again by the
//! time the next processed block does. The fade is **linear**, not equal power:
//! see [`splice`] for why the note's word is the wrong law here.
//!
//! # Latency
//!
//! A plugin that looks ahead answers back late. Because a chain is rendered
//! whole rather than pulled a block at a time, compensating is free: the run is
//! given the chain's summed latency in extra silent frames at the end so the
//! delayed tail actually comes out, and the caller places the processed sound
//! that many frames **earlier** so the wet lands where the dry did. A
//! lookahead limiter then just works.

use std::sync::Arc;

use super::ParamId;

/// Frames in one block — fixed at 512 (~10.7 ms at the session's 48 kHz), the
/// same control rate the Volume envelope already uses (K-172).
pub const AUDIO_BLOCK_FRAMES: usize = 512;

/// Channels. v1 hosts stereo effect plugins only
/// (docs/impl/audio-plugins.md §4).
pub const AUDIO_CHANNELS: usize = 2;

/// Interleaved samples in one block.
pub const AUDIO_BLOCK_SAMPLES: usize = AUDIO_BLOCK_FRAMES * AUDIO_CHANNELS;

/// The splice ramp either side of a dry stretch: 5 ms at 48 kHz.
pub const AUDIO_SPLICE_FRAMES: usize = 240;

/// One live audio effect the mixer can play sound through.
///
/// Implemented by the plugin host (`lumit-aplug`'s brokered CLAP instance
/// today, VST3's the same way in AP4) and by whatever a test wants to stand in
/// its place. Nothing in this crate knows which.
///
/// **Never called with a lock held, and never from a rebuild path**: a block
/// may block on somebody else's code in somebody else's process.
pub trait AudioProcessor: Send + Sync {
    /// One block. `input` and `output` are [`AUDIO_BLOCK_SAMPLES`] of
    /// interleaved stereo, always **separate buffers** — in-place is where
    /// plugin bugs live.
    ///
    /// `values` is what every automated row holds at this block's first frame;
    /// the whole list is handed over every block rather than a diff, because a
    /// broker that has just restarted replayed only the values it was created
    /// with and a diff would leave it stale for ever.
    ///
    /// `steady` is the running frame count since the chain started.
    ///
    /// `false` means the block did not come back, and the caller ships it dry.
    /// Never a `Result`: there is exactly one thing to do about any reason, and
    /// the sentence for the badge is the host's own business.
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        steady: i64,
    ) -> bool;

    /// Frames of delay this effect adds. Nought for everything that answers in
    /// the moment, which is most things.
    fn latency(&self) -> u32 {
        0
    }

    /// The sentence the most recent refused block carried, where the host kept
    /// one — what the calm badge shows underneath its reason (AP5,
    /// docs/12 §2.3). `None` for a processor that has never refused a block,
    /// and for one with no words about it, which the badge draws as the reason
    /// alone.
    fn last_error(&self) -> Option<String> {
        None
    }
}

/// One link of a chain: a live processor and what its rows hold, block by
/// block.
pub struct ChainLink {
    pub processor: Arc<dyn AudioProcessor>,
    /// `values[b]` is what block `b` is handed; the last entry holds past the
    /// end, exactly as a gain envelope's last control point does. A link whose
    /// rows are all static and unwired carries **one** entry, which is what
    /// makes an un-automated plugin cost no per-block work at all.
    pub values: Vec<Vec<(ParamId, f64)>>,
}

/// What one run of a chain produced.
pub struct ChainOutput {
    /// Interleaved stereo, `frames + latency` frames long: the input's own
    /// length plus the tail the compensation asked for.
    pub samples: Vec<f32>,
    /// How many blocks came back dry — nought is the ordinary answer, and
    /// anything else is what badges the layer.
    pub dry_blocks: usize,
    /// The same count told per link, in chain order — which is what lets the
    /// badge land on the effect that refused rather than on the whole rack
    /// (AP5). Empty for an empty chain.
    pub dry_by_link: Vec<usize>,
    /// The chain's summed latency in frames. The caller places the processed
    /// sound this many frames earlier.
    pub latency: u32,
}

/// Run `input` (interleaved stereo) through `chain`, in order.
///
/// An empty chain hands the input straight back, which is what keeps a layer
/// with no audio effect byte-identical to what it was before this module
/// existed.
#[must_use]
pub fn run_chain(chain: &[ChainLink], input: &[f32]) -> ChainOutput {
    let frames = input.len() / AUDIO_CHANNELS;
    let latency: u32 = chain
        .iter()
        .map(|link| link.processor.latency())
        .fold(0u32, u32::saturating_add);
    if chain.is_empty() || frames == 0 {
        return ChainOutput {
            samples: input.to_vec(),
            dry_blocks: 0,
            dry_by_link: vec![0; chain.len()],
            latency: 0,
        };
    }

    // The tail the compensation needs, then whole blocks: a plugin cannot be
    // asked for half a block, and the last one of a layer is simply silent
    // past the end.
    let wanted = frames + latency as usize;
    let blocks = wanted.div_ceil(AUDIO_BLOCK_FRAMES);
    let padded = blocks * AUDIO_BLOCK_SAMPLES;

    let mut src = vec![0.0f32; padded];
    let head = (frames * AUDIO_CHANNELS).min(padded);
    src[..head].copy_from_slice(&input[..head]);
    let mut dst = vec![0.0f32; padded];
    let mut wet = vec![false; blocks];
    let mut dry_blocks = 0usize;
    let mut dry_by_link = vec![0usize; chain.len()];

    for (li, link) in chain.iter().enumerate() {
        for (b, flag) in wet.iter_mut().enumerate() {
            let at = b * AUDIO_BLOCK_SAMPLES;
            let end = at + AUDIO_BLOCK_SAMPLES;
            let values = link
                .values
                .get(b)
                .or_else(|| link.values.last())
                .map_or(&[][..], Vec::as_slice);
            let steady = (b * AUDIO_BLOCK_FRAMES) as i64;
            *flag = link
                .processor
                .process(&src[at..end], &mut dst[at..end], values, steady);
            if !*flag {
                dst[at..end].copy_from_slice(&src[at..end]);
                dry_blocks += 1;
                dry_by_link[li] += 1;
            }
        }
        splice(&mut dst, &src, &wet);
        std::mem::swap(&mut src, &mut dst);
    }

    src.truncate(wanted * AUDIO_CHANNELS);
    ChainOutput {
        samples: src,
        dry_blocks,
        dry_by_link,
        latency,
    }
}

/// Crossfade the wet signal to and from the dry one across every edge of a dry
/// stretch, so a block that did not come back does not click.
///
/// The ramp lives on the **wet** side of each boundary: the tail of the last
/// processed block before a dry one, and the head of the first processed block
/// after it. Inside the dry stretch `out` already *is* `dry`, so there is
/// nothing there to blend.
///
/// **Linear, not equal power**, which is where this departs from
/// docs/impl/audio-plugins.md §3's word (K-700 records the reversal).
///
/// Equal power is the law for two *uncorrelated* signals — two different shots
/// across a dissolve, which is why [`crate::sequence`]'s clip crossfades use it.
/// These two are the **same sound** differently treated, and for a correlated
/// pair the sine/cosine weights sum to as much as √2 in the middle: a ramp that
/// exists to hide a click would put a 3 dB swell there instead. Worse, a plugin
/// that happens to be a passthrough would have its dry block *changed* by the
/// ramp around it, which is plainly not what "shipped dry" means. Linear
/// weights sum to one at every point, so an identical pair passes through
/// untouched and a different one moves smoothly.
fn splice(out: &mut [f32], dry: &[f32], wet: &[bool]) {
    for b in 1..wet.len() {
        if wet[b] == wet[b - 1] {
            continue;
        }
        // Entering a dry stretch, ramp out over the tail before it; leaving
        // one, ramp in over the head after it.
        let (first, rising) = if wet[b] {
            (b * AUDIO_BLOCK_FRAMES, true)
        } else {
            (b * AUDIO_BLOCK_FRAMES - AUDIO_SPLICE_FRAMES, false)
        };
        for i in 0..AUDIO_SPLICE_FRAMES {
            // The wet share: nought at the dry end of the ramp, one at the
            // other.
            let x = if rising {
                i as f32 / AUDIO_SPLICE_FRAMES as f32
            } else {
                1.0 - i as f32 / AUDIO_SPLICE_FRAMES as f32
            };
            let (w, d) = (x, 1.0 - x);
            let at = (first + i) * AUDIO_CHANNELS;
            for c in 0..AUDIO_CHANNELS {
                let Some(slot) = out.get_mut(at + c) else {
                    continue;
                };
                let Some(&plain) = dry.get(at + c) else {
                    continue;
                };
                *slot = *slot * w + plain * d;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A processor that multiplies by whatever its one row holds, and can be
    /// told to fail on one block.
    struct Gain {
        row: ParamId,
        fail_on: Option<usize>,
        latency: u32,
        seen: AtomicUsize,
    }

    impl Gain {
        fn new(fail_on: Option<usize>) -> Self {
            Self {
                row: ParamId::new("p1"),
                fail_on,
                latency: 0,
                seen: AtomicUsize::new(0),
            }
        }
    }

    impl AudioProcessor for Gain {
        fn process(
            &self,
            input: &[f32],
            output: &mut [f32],
            values: &[(ParamId, f64)],
            steady: i64,
        ) -> bool {
            self.seen.fetch_add(1, Ordering::Relaxed);
            let block = steady as usize / AUDIO_BLOCK_FRAMES;
            if self.fail_on == Some(block) {
                return false;
            }
            let g = values
                .iter()
                .find(|(id, _)| *id == self.row)
                .map_or(1.0, |(_, v)| *v) as f32;
            for (o, i) in output.iter_mut().zip(input) {
                *o = i * g;
            }
            true
        }

        fn latency(&self) -> u32 {
            self.latency
        }
    }

    fn link(processor: Gain, values: Vec<Vec<(ParamId, f64)>>) -> ChainLink {
        ChainLink {
            processor: Arc::new(processor),
            values,
        }
    }

    #[test]
    fn an_empty_chain_hands_the_input_straight_back() {
        let input = vec![0.5f32; 8];
        let out = run_chain(&[], &input);
        assert_eq!(out.samples, input);
        assert_eq!(out.dry_blocks, 0);
        assert_eq!(out.latency, 0);
    }

    #[test]
    fn a_static_row_holds_past_its_one_entry() {
        // Three blocks of input, one value: every block is handed the same one.
        let frames = AUDIO_BLOCK_FRAMES * 3;
        let input = vec![0.25f32; frames * AUDIO_CHANNELS];
        let gain = Gain::new(None);
        let out = run_chain(&[link(gain, vec![vec![(ParamId::new("p1"), 2.0)]])], &input);
        assert_eq!(out.samples.len(), input.len());
        assert!(out.samples.iter().all(|s| (*s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn each_block_is_handed_its_own_value_and_two_runs_agree() {
        // A sweep: block b is multiplied by b.
        let blocks = 4;
        let frames = AUDIO_BLOCK_FRAMES * blocks;
        let input = vec![1.0f32; frames * AUDIO_CHANNELS];
        let values: Vec<Vec<(ParamId, f64)>> = (0..blocks)
            .map(|b| vec![(ParamId::new("p1"), b as f64)])
            .collect();
        let first = run_chain(&[link(Gain::new(None), values.clone())], &input);
        let again = run_chain(&[link(Gain::new(None), values)], &input);
        assert_eq!(first.samples, again.samples, "the run is deterministic");
        for b in 0..blocks {
            let at = b * AUDIO_BLOCK_SAMPLES;
            assert!(
                (first.samples[at] - b as f32).abs() < 1e-6,
                "block {b} took the wrong value"
            );
        }
    }

    #[test]
    fn a_refused_block_ships_dry_with_a_ramp_either_side() {
        let blocks = 3;
        let frames = AUDIO_BLOCK_FRAMES * blocks;
        let input = vec![1.0f32; frames * AUDIO_CHANNELS];
        // Doubling, except block 1, which is refused.
        let out = run_chain(
            &[link(
                Gain::new(Some(1)),
                vec![vec![(ParamId::new("p1"), 2.0)]],
            )],
            &input,
        );
        assert_eq!(out.dry_blocks, 1);
        // The dry block itself is the input, unchanged.
        let dry_at = AUDIO_BLOCK_SAMPLES + AUDIO_SPLICE_FRAMES * AUDIO_CHANNELS;
        assert!((out.samples[dry_at] - 1.0).abs() < 1e-6, "the block is dry");
        // Block 0 is wet where the ramp has not reached…
        assert!((out.samples[0] - 2.0).abs() < 1e-6);
        // …and has come all the way down to dry by the splice.
        let edge = (AUDIO_BLOCK_FRAMES - 1) * AUDIO_CHANNELS;
        assert!(
            (out.samples[edge] - 1.0).abs() < 0.02,
            "the wet tail reaches the splice already dry, got {}",
            out.samples[edge]
        );
        // Block 2 comes back wet, ramping up from the splice rather than
        // jumping there.
        let back = 2 * AUDIO_BLOCK_SAMPLES;
        assert!(
            (out.samples[back] - 1.0).abs() < 0.02,
            "the wet head starts at the dry level"
        );
        let settled = back + AUDIO_SPLICE_FRAMES * AUDIO_CHANNELS;
        assert!((out.samples[settled] - 2.0).abs() < 1e-6, "and reaches wet");
        // Nothing jumps: the biggest step between neighbouring frames is small.
        let worst = out
            .samples
            .chunks_exact(AUDIO_CHANNELS)
            .zip(out.samples.chunks_exact(AUDIO_CHANNELS).skip(1))
            .map(|(a, b)| (b[0] - a[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.02, "the splice clicks: worst step {worst}");
    }

    /// **A ramp between two identical signals changes nothing** — the property
    /// an equal-power crossfade would break, and the reason `splice` is linear
    /// (K-700). A passthrough plugin that dies mid-run must leave the sound
    /// exactly as it found it, ramps and all.
    #[test]
    fn a_dry_splice_through_a_passthrough_leaves_the_sound_alone() {
        struct Passthrough(Option<usize>);
        impl AudioProcessor for Passthrough {
            fn process(
                &self,
                input: &[f32],
                output: &mut [f32],
                _values: &[(ParamId, f64)],
                steady: i64,
            ) -> bool {
                if self.0 == Some(steady as usize / AUDIO_BLOCK_FRAMES) {
                    return false;
                }
                output.copy_from_slice(input);
                true
            }
        }
        let input = vec![0.25f32; AUDIO_BLOCK_FRAMES * 3 * AUDIO_CHANNELS];
        let out = run_chain(
            &[ChainLink {
                processor: Arc::new(Passthrough(Some(1))),
                values: Vec::new(),
            }],
            &input,
        );
        assert_eq!(out.dry_blocks, 1);
        assert!(
            out.samples.iter().all(|s| (*s - 0.25).abs() < 1e-6),
            "the ramp must not put a swell where the two signals are the same"
        );
    }

    #[test]
    fn latency_buys_a_tail_and_is_summed_across_the_chain() {
        let frames = AUDIO_BLOCK_FRAMES;
        let input = vec![1.0f32; frames * AUDIO_CHANNELS];
        let mut a = Gain::new(None);
        a.latency = 100;
        let mut b = Gain::new(None);
        b.latency = 44;
        let out = run_chain(
            &[
                link(a, vec![vec![(ParamId::new("p1"), 1.0)]]),
                link(b, vec![vec![(ParamId::new("p1"), 1.0)]]),
            ],
            &input,
        );
        assert_eq!(out.latency, 144);
        assert_eq!(
            out.samples.len(),
            (frames + 144) * AUDIO_CHANNELS,
            "the run is long enough for the delayed tail to come out"
        );
    }

    #[test]
    fn the_links_run_in_order_and_compose() {
        let frames = 8;
        let input = vec![1.0f32; frames * AUDIO_CHANNELS];
        let out = run_chain(
            &[
                link(Gain::new(None), vec![vec![(ParamId::new("p1"), 3.0)]]),
                link(Gain::new(None), vec![vec![(ParamId::new("p1"), 0.5)]]),
            ],
            &input,
        );
        assert!((out.samples[0] - 1.5).abs() < 1e-6);
    }
}
