//! A strip's or a bus's rack as a stream: its audio effects run a block at a
//! time on the node's whole sum, ahead of the playhead on the rack thread
//! and in order from frame 0 in an export ([`lumit_audio::rack`]).
//!
//! The effects open exactly as a bake opens them ([`super::bake::open_links`])
//! and each block hands every link its rows' values at that block, read by
//! the same function a bake reads them with, so a rack streamed and a rack
//! baked are handed the same numbers. A link that refuses a block ships it
//! dry with a short ramp at the splice, and its badge is filed as a bake
//! files it.
//!
//! Opened and run on the rack thread or an export's thread, never the
//! callback. A block may wait on a plugin in another process.

use lumit_audio::rack::{RackOpener, RackStream};
use lumit_core::fx::audio_chain::{AudioProcessor, Denormals, AUDIO_SPLICE_FRAMES};
use lumit_core::fx::params::ParamId;
use lumit_core::fx::registry::EffectDef;
use lumit_core::model::EffectInstance;
use std::any::Any;
use std::sync::Arc;
use uuid::Uuid;

use super::bake::{file_badge, open_links, rows_of};
use super::jobs::AudioChain;

/// Whether any switched-on effect in `effects` sounds: a hosted audio plugin
/// or a built-in audio effect. A stack of picture effects is no rack.
pub(crate) fn sounds(effects: &[EffectInstance]) -> bool {
    effects.iter().filter(|e| e.enabled).any(audio_effect)
}

/// Whether `e` is an audio effect, hosted or built in.
fn audio_effect(e: &EffectInstance) -> bool {
    lumit_core::fx::builtins::audio_plugin_id(&e.effect.match_name).is_some()
        || lumit_core::fx::catalogue::BUILTIN_DEFS
            .get(&e.effect.match_name)
            .is_some_and(|def| def.schema().category == lumit_core::fx::schema::FxCategory::Audio)
}

/// What opens a node's rack: its chain, and the rack's structure as a
/// number ([`RackOpener::identity`]).
pub struct NodeRack {
    chain: Arc<AudioChain>,
    identity: u64,
}

impl NodeRack {
    /// The rack of `chain`. `stamp` is whatever else the effects' sound
    /// depends on that the chain does not carry (the caller's
    /// [`super::live::RackBakes`] stamp), so a change there opens the
    /// stream afresh.
    #[must_use]
    pub fn new(chain: Arc<AudioChain>, stamp: u64) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(&stamp.to_le_bytes());
        h.update(&crate::voice::stamp().to_le_bytes());
        for e in chain.effects.iter().filter(|e| audio_effect(e)) {
            h.update(e.id.as_bytes());
            h.update(e.effect.match_name.as_bytes());
            h.update(&[u8::from(e.enabled)]);
            if let Some(state) = e.plugin_state_bytes() {
                h.update(&(state.len() as u64).to_le_bytes());
                h.update(&state);
            }
        }
        let mut first = [0u8; 8];
        first.copy_from_slice(&h.finalize().as_bytes()[..8]);
        Self {
            chain,
            identity: u64::from_le_bytes(first),
        }
    }

    /// Whether one of its built-in effects has to hear its whole input
    /// before its first block, so the rack cannot stream from the playhead.
    /// Only built-ins are asked, which costs nothing to open: no hosted
    /// plugin learns from its whole input.
    #[must_use]
    pub fn bakes_whole(&self, rate: u32) -> bool {
        self.chain.effects.iter().filter(|e| e.enabled).any(|e| {
            if lumit_core::fx::builtins::audio_plugin_id(&e.effect.match_name).is_some() {
                return false;
            }
            let Some(def) = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&e.effect.match_name)
            else {
                return false;
            };
            let first = rows_of(e, def).map_or_else(Vec::new, |r| r.at(&self.chain, e.id, 0, rate));
            crate::voice::open_audio(e, def, &first, rate, true).is_some_and(|p| p.bakes_whole())
        })
    }

    /// The chain it opens.
    #[must_use]
    pub fn chain(&self) -> &Arc<AudioChain> {
        &self.chain
    }
}

impl RackOpener for NodeRack {
    fn identity(&self) -> u64 {
        self.identity
    }

    fn open(&self, rate: u32, offline: bool) -> Option<Box<dyn RackStream>> {
        let links: Vec<Link> = open_links(&self.chain, 0, rate, offline)
            .into_iter()
            .map(|(instance, def, processor)| Link {
                id: instance.id,
                def,
                processor,
                held: None,
                wet: true,
                last: [0.0; 2],
                refused: false,
                filed: None,
            })
            .collect();
        if links.is_empty() {
            return None;
        }
        let mut stream = NodeStream {
            chain: Arc::clone(&self.chain),
            links,
            rate,
            steady: 0,
            src: Vec::new(),
            dst: Vec::new(),
        };
        stream.hold_values();
        Some(Box::new(stream))
    }

    fn update(&self, stream: &mut dyn RackStream) -> bool {
        let Some(stream) = stream.as_any_mut().downcast_mut::<NodeStream>() else {
            return false;
        };
        stream.chain = Arc::clone(&self.chain);
        stream.hold_values();
        true
    }
}

/// One opened effect of a stream.
struct Link {
    id: Uuid,
    def: &'static dyn EffectDef,
    processor: Arc<dyn AudioProcessor>,
    /// Its rows' values when none of them moves, read once rather than per
    /// block.
    held: Option<Vec<(ParamId, f64)>>,
    /// Whether the last block came back, and its last frame: where a splice
    /// into a dry block ramps from.
    wet: bool,
    last: [f32; 2],
    /// Whether it has refused a block since it opened, and what its badge
    /// says now.
    refused: bool,
    filed: Option<bool>,
}

/// An opened rack, run a block at a time.
struct NodeStream {
    /// Where the rows' values are read: handed afresh by every
    /// [`RackOpener::update`].
    chain: Arc<AudioChain>,
    links: Vec<Link>,
    rate: u32,
    /// Frames processed since it opened.
    steady: i64,
    src: Vec<f32>,
    dst: Vec<f32>,
}

impl NodeStream {
    /// Read the values of every link whose rows do not move.
    fn hold_values(&mut self) {
        for link in &mut self.links {
            let instance = self.chain.effects.iter().find(|e| e.id == link.id);
            link.held = instance.and_then(|instance| {
                let rows = rows_of(instance, link.def);
                match rows {
                    Some(rows) if rows.moves(&self.chain, link.id) => None,
                    Some(rows) => Some(rows.at(&self.chain, link.id, 0, self.rate)),
                    None => Some(Vec::new()),
                }
            });
        }
    }
}

impl RackStream for NodeStream {
    fn latency(&self) -> u32 {
        self.links
            .iter()
            .map(|l| l.processor.latency())
            .fold(0u32, u32::saturating_add)
    }

    fn tail(&self) -> u32 {
        self.links
            .iter()
            .map(|l| l.processor.tail())
            .fold(0u32, u32::saturating_add)
    }

    fn process(&mut self, input: &[f32], output: &mut [f32], at: i64) -> bool {
        let _denormals = Denormals::on();
        let n = input.len().min(output.len()) / 2 * 2;
        let NodeStream {
            chain,
            links,
            rate,
            steady,
            src,
            dst,
        } = self;
        src.clear();
        src.extend_from_slice(&input[..n]);
        dst.clear();
        dst.resize(n, 0.0);
        for link in links.iter_mut() {
            // A sample that is not a number would stay in an effect's state
            // for the rest of the run, so each link reads it as nought.
            for s in src.iter_mut() {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }
            let values = match &link.held {
                Some(values) => std::borrow::Cow::Borrowed(values),
                None => std::borrow::Cow::Owned(
                    chain
                        .effects
                        .iter()
                        .find(|e| e.id == link.id)
                        .and_then(|e| rows_of(e, link.def))
                        .map_or_else(Vec::new, |rows| rows.at(chain, link.id, at, *rate)),
                ),
            };
            let wet = link.processor.process(src, dst, &values, *steady);
            if wet {
                if !link.wet {
                    // Out of a dry stretch: ramp the wet sound in over the
                    // head of this block, as a bake does.
                    splice_in(dst, src);
                }
            } else {
                dst.copy_from_slice(src);
                if link.wet && *steady > 0 {
                    // Into one: ramp from where the wet sound stopped, since
                    // the block it would have faded over has gone out.
                    let last = link.last;
                    for (i, f) in dst
                        .chunks_exact_mut(2)
                        .take(AUDIO_SPLICE_FRAMES)
                        .enumerate()
                    {
                        let x = i as f32 / AUDIO_SPLICE_FRAMES as f32;
                        f[0] = last[0] * (1.0 - x) + f[0] * x;
                        f[1] = last[1] * (1.0 - x) + f[1] * x;
                    }
                }
                link.refused = true;
            }
            link.wet = wet;
            if let [.., l, r] = dst.as_slice() {
                link.last = [*l, *r];
            }
            if link.filed != Some(link.refused) {
                file_badge(link.id, link.processor.as_ref(), link.refused);
                link.filed = Some(link.refused);
            }
            std::mem::swap(src, dst);
        }
        output[..n].copy_from_slice(&src[..n]);
        *steady += (n / 2) as i64;
        true
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Crossfade the head of `out` in from `dry`, linearly across
/// [`AUDIO_SPLICE_FRAMES`].
fn splice_in(out: &mut [f32], dry: &[f32]) {
    for (i, (o, d)) in out
        .chunks_exact_mut(2)
        .zip(dry.chunks_exact(2))
        .take(AUDIO_SPLICE_FRAMES)
        .enumerate()
    {
        let w = i as f32 / AUDIO_SPLICE_FRAMES as f32;
        o[0] = o[0] * w + d[0] * (1.0 - w);
        o[1] = o[1] * w + d[1] * (1.0 - w);
    }
}
