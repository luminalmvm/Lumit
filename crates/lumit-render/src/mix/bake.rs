//! The racks and the gains baked for one job: what its effects make of its
//! sound, and its Volume and Pan as a constant or an envelope.

use std::sync::Arc;
use uuid::Uuid;

use super::jobs::{AudioChain, AudioJob, Carrier};

/// Open the chain and play `samples` (interleaved stereo, the job's placed
/// span) through it — the **one** function the live plan and the export both
/// call, which is what makes preview == export a fact about the code rather
/// than an argument about it.
///
/// `start_frame` is where the span lands on the mixed timeline, and is what the
/// per-block parameter values are read at. `offline` says this is an export: no
/// deadline, and the plugin may take its slower, better path.
///
/// `None` when the layer's stack holds nothing the catalogue will open as
/// audio, which is the ordinary answer and the one that leaves the decoded
/// buffer untouched. Otherwise the processed span and the chain's summed
/// latency, in frames — the caller places the sound that many frames **earlier**
/// so the wet lands where the dry did.
///
/// The span that comes back may be **longer** than the one that went in: a
/// chain with a tail (a reverb's decay, an echo's repeats) runs on past its
/// input, and the caller places the whole of what it is given, so the decay
/// simply sums with whatever follows.
///
/// ponytail: the whole span is processed here, at plan-build time, rather than
/// a block at a time into a lookahead ring on a chain worker
/// (docs/impl/audio-plugins.md §3). The ring exists to keep the realtime
/// callback off another process; a span already rendered keeps it off even more
/// firmly. The plan streams its files now ([`lumit_audio::stream`]) and a
/// racked clip is the one thing in it that is still held whole: its processed
/// span stays in memory for as long as the plan does, outside the block
/// budget, and [`super::live::RackBakes`] keeps it from being made again on every edit.
/// The ring is the upgrade, with this function as the block loop it wraps,
/// when a cut carries enough racked sound for that memory to matter.
#[must_use]
pub fn chain_bake(
    chain: &AudioChain,
    samples: &[f32],
    start_frame: i64,
    rate: u32,
    offline: bool,
) -> Option<(Vec<f32>, u32)> {
    use lumit_core::fx::audio_chain::AUDIO_BLOCK_FRAMES;

    let frames = samples.len() / 2;
    if frames == 0 {
        return None;
    }
    // Open first, bake second. Opening answers the question "are you audio?",
    // and only once every link has answered is the chain's summed latency —
    // and therefore how many blocks the run is — known. Baking a whole
    // envelope for a blur that turns out not to be a plugin would be work for
    // nothing, so opening is handed one time point rather than the envelope.
    let mut opened = Vec::new();
    for instance in chain.effects.iter().filter(|e| e.enabled) {
        let Some(def) = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&instance.effect.match_name)
        else {
            continue;
        };
        let first = bake_values(chain, instance, def, start_frame, 1, rate)
            .pop()
            .unwrap_or_default();
        if let Some(processor) = crate::voice::open_audio(instance, def, &first, rate, offline) {
            opened.push((instance, def, processor));
        }
    }
    if opened.is_empty() {
        return None;
    }
    let latency = opened
        .iter()
        .map(|(_, _, processor)| processor.latency() as usize)
        .sum::<usize>();
    // The tail belongs in the block count too: `run_chain` runs the extra
    // frames, and a row automated past the input's end still wants a value for
    // every one of them.
    let tail = opened
        .iter()
        .map(|(_, _, processor)| processor.tail() as usize)
        .sum::<usize>();
    let blocks = (frames + latency + tail).div_ceil(AUDIO_BLOCK_FRAMES);
    let ids: Vec<Uuid> = opened.iter().map(|(instance, _, _)| instance.id).collect();
    let links: Vec<lumit_core::fx::audio_chain::ChainLink> = opened
        .into_iter()
        .map(
            |(instance, def, processor)| lumit_core::fx::audio_chain::ChainLink {
                values: bake_values(chain, instance, def, start_frame, blocks, rate),
                processor,
            },
        )
        .collect();
    let out = lumit_core::fx::audio_chain::run_chain(&links, samples);
    // The badge, per link (AP5, docs/12 §2.3): a link that shipped any block
    // dry files the host's own sentence against *its* instance — the same
    // table an OFX frame files into, read by the same `badge_of` — and a link
    // whose every block came back takes any stale badge off. Filed here, on
    // the bake, because this is the one place that knows which link refused.
    for ((id, dry), link) in ids.iter().zip(&out.dry_by_link).zip(&links) {
        let sentence = (*dry > 0).then(|| {
            link.processor
                .last_error()
                .unwrap_or_else(|| "the plugin did not process this sound".to_owned())
        });
        crate::gpufx::ofx::note(*id, sentence);
    }
    Some((out.samples, out.latency))
}

/// Run a job's **two** chains over its placed span: the clip's own rack
/// first, then the row's on what came out of it
/// (docs/impl/audio-timeline.md §4).
///
/// The one function both mixers call, for the same reason [`chain_bake`] is:
/// the order of the racks and the sum of their latencies are facts about the
/// code rather than something the preview and the export each decide. `None`
/// when neither chain opens anything as audio, which leaves the decoded buffer
/// exactly as it was.
///
/// The row's chain is read at `start_frame` less the clip chain's latency,
/// because that is where the sound it is handed now begins; the returned
/// latency is the two summed, and the caller places the run that many frames
/// earlier.
#[must_use]
pub fn job_bake(
    job: &AudioJob,
    samples: &[f32],
    start_frame: i64,
    rate: u32,
    offline: bool,
) -> Option<(Vec<f32>, u32)> {
    let clip = job
        .clip_chain
        .as_ref()
        .and_then(|chain| chain_bake(chain, samples, start_frame, rate, offline));
    let (dry, head): (&[f32], u32) = match &clip {
        Some((wet, latency)) => (wet, *latency),
        None => (samples, 0),
    };
    let layer = job
        .chain
        .as_ref()
        .and_then(|chain| chain_bake(chain, dry, start_frame - i64::from(head), rate, offline));
    match layer {
        Some((wet, latency)) => Some((wet, latency.saturating_add(head))),
        None => clip,
    }
}

/// What one effect instance's rows hold at each block start.
///
/// A row reads its own keyframes unless a wire feeds it, in which case it reads
/// the wire — the same substitution the picture's resolve makes, held to the
/// same declared hard range, so a plugin parameter is driven
/// like everything else. An instance with nothing animated and nothing wired
/// bakes **one** entry, which the chain then holds past the end: an
/// un-automated plugin costs no per-block work at all.
///
/// ponytail: the driver walk is given no [`lumit_core::fx::registry::AudioTap`], so an
/// *Audio level* driver wired into a plugin parameter reads nothing here.
/// Handing it the real tap would mix a window of the comp per block — tens of
/// thousands of mixes for a song. Bake the tap's levels once at control rate
/// and index them, when a duck-driven plugin is a thing somebody asks for.
fn bake_values(
    chain: &AudioChain,
    instance: &lumit_core::model::EffectInstance,
    def: &'static dyn lumit_core::fx::registry::EffectDef,
    start_frame: i64,
    blocks: usize,
    rate: u32,
) -> Vec<Vec<(lumit_core::fx::params::ParamId, f64)>> {
    use lumit_core::fx::{audio_chain::AUDIO_BLOCK_FRAMES, params::ParamId, resolved::hard_range};
    use lumit_core::graph::{InputRef, NodeRef};
    use lumit_core::model::EffectValue;

    /// One automatable row: what it is called, what it holds, and how far it
    /// may be pushed.
    struct Row<'a> {
        id: ParamId,
        property: std::borrow::Cow<'a, lumit_core::anim::Property>,
        hard: (Option<f64>, Option<f64>),
    }

    let rows: Vec<Row<'_>> = def
        .schema()
        .params
        .iter()
        .filter_map(|row| {
            let held = instance.params.iter().find(|p| p.id == row.id)?;
            let property = match &held.value {
                EffectValue::Float(property) => std::borrow::Cow::Borrowed(property),
                // A switch goes to a plugin as nought or one.
                EffectValue::Bool(on) => std::borrow::Cow::Owned(
                    lumit_core::anim::Property::fixed(f64::from(u8::from(*on))),
                ),
                // A list goes as the index of what is picked.
                EffectValue::Choice(index) => {
                    std::borrow::Cow::Owned(lumit_core::anim::Property::fixed(f64::from(*index)))
                }
                _ => return None,
            };
            Some(Row {
                id: ParamId::new(row.id),
                property,
                hard: hard_range(&row.kind),
            })
        })
        .collect();
    if rows.is_empty() {
        return Vec::new();
    }
    let node = NodeRef::Effect(instance.id);
    let wired =
        chain.graph.edges.iter().any(
            |edge| matches!(&edge.to, InputRef::Param { node: target, .. } if *target == node),
        );
    let animated = wired || rows.iter().any(|row| row.property.is_animated());
    let n = if animated { blocks.max(1) } else { 1 };

    (0..n)
        .map(|block| {
            let t = (start_frame + (block * AUDIO_BLOCK_FRAMES) as i64) as f64 / f64::from(rate);
            let lt = t - chain.offset_s;
            let drivers = if wired {
                let context = Arc::new(lumit_core::expression::ExpressionContext {
                    document: Arc::clone(&chain.doc),
                    comp: Some(chain.comp),
                    layer: Some(chain.layer),
                    comp_time: t - chain.base_s,
                    current_depth: 0,
                    inputs: None,
                });
                lumit_core::fx::drivers::resolve_drivers(&chain.graph, lt, context, None)
            } else {
                lumit_core::fx::drivers::ResolvedDrivers::default()
            };
            rows.iter()
                .map(|row| {
                    let value = drivers
                        .param(node, row.id)
                        .map(|v| f64::from(v.as_f32()))
                        .unwrap_or_else(|| row.property.value_at(lt));
                    // `max`/`min` rather than `clamp`, which panics on a
                    // reversed pair (14-ENGINEERING-RULES §4).
                    let value = value
                        .max(row.hard.0.unwrap_or(f64::NEG_INFINITY))
                        .min(row.hard.1.unwrap_or(f64::INFINITY));
                    (row.id, value)
                })
                .collect()
        })
        .collect()
}

/// Bake one job's **Volume and Pan** — its own properties times every
/// carrier's — for its placed span: `([left, right] constant gain,
/// envelope)`. All-static chains are exactly their constant product
/// (envelope None); any animated link bakes the whole chain to a ~10 ms
/// control-rate curve, each property sampled in its own layer time
/// (`lt = comp time − its offset`).
///
/// **One stage for both**. A balance is a pair of per-channel gains,
/// so it multiplies into the Volume's gain rather than needing a stage of its
/// own — which is also what makes a Precomp layer's balance compose with the
/// balances inside it, channel by channel.
pub fn volume_bake(
    job: &AudioJob,
    start_frame: i64,
    len: usize,
    rate: u32,
) -> ([f32; 2], Option<lumit_audio::mix::GainEnvelope>) {
    gain_bake(Some(job), &job.carriers, start_frame, len, rate)
}

/// The gain law itself: `own`'s Volume, Pan and fade where a job's own sound
/// is being placed, times `carriers`.
///
/// The bus stage splits one job's chain of gains in two, the carriers inside
/// the bus riding into the sum and the ones outside riding on what the rack
/// gives back, so both halves are asked for here rather than each doing its
/// own arithmetic. `None` for `own` is a summed bus, whose own Volume and Pan were
/// spent on the sources inside it.
pub(super) fn gain_bake(
    own: Option<&AudioJob>,
    carriers: &[Carrier],
    start_frame: i64,
    len: usize,
    rate: u32,
) -> ([f32; 2], Option<lumit_audio::mix::GainEnvelope>) {
    let gain_at = |t: f64| {
        let mut g = 1.0;
        let mut lr = [1.0f32, 1.0];
        if let Some(job) = own {
            // A wired Volume overrides its keyframes, exactly as a wired effect
            // parameter does; a broken or bypassed chain answers `None`
            // and the keyframes come back.
            let volume_db = job
                .driven
                .as_ref()
                .and_then(|d| d.db_at(t))
                .unwrap_or_else(|| job.volume.value_at(t - job.offset_s));
            g = lumit_audio::mix::db_to_gain(volume_db);
            lr = lumit_audio::mix::pan_gains(job.pan.value_at(t - job.offset_s));
            // A clip's own crossfade ramps multiply in too: they are the
            // join's, not the layer's, and they ride on whatever the Volume is
            // doing.
            if let Some(fade) = &job.fade {
                g *= fade.gain_at(t);
            }
        }
        for c in carriers {
            g *= lumit_audio::mix::db_to_gain(c.volume.value_at(t - c.offset_s));
            if let Some(fade) = &c.fade {
                g *= fade.gain_at(t);
            }
            let cp = lumit_audio::mix::pan_gains(c.pan.value_at(t - c.offset_s));
            lr[0] *= cp[0];
            lr[1] *= cp[1];
        }
        [lr[0] * g, lr[1] * g]
    };
    let animated = own.is_some_and(|job| {
        job.volume.is_animated()
            || job.pan.is_animated()
            || job.fade.is_some_and(|f| f.is_active())
            // A driven Volume follows the sound, which moves whether or not any
            // keyframe does — always an envelope, never a constant.
            || job.driven.is_some()
    }) || carriers.iter().any(|c| {
        c.volume.is_animated() || c.pan.is_animated() || c.fade.is_some_and(|f| f.is_active())
    });
    if !animated {
        return (gain_at(0.0), None);
    }
    let stride = (rate / 100).max(1);
    let n = len / stride as usize + 2;
    let points = (0..n)
        .map(|p| {
            let t = (start_frame + p as i64 * i64::from(stride)) as f64 / f64::from(rate);
            gain_at(t)
        })
        .collect();
    (
        [1.0, 1.0],
        Some(lumit_audio::mix::GainEnvelope { stride, points }),
    )
}
