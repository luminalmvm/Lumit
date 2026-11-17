//! The comp's desk, built from its jobs: a node per sounding layer and per
//! Bus layer of the comp being mixed, each with its fader, rack, sends,
//! output and duck, and the clips summed into them (docs/09-AUDIO.md).
//!
//! # In plain terms
//!
//! The jobs walk says what sounds and where. This turns that into the plan
//! [`lumit_audio::plan::MixPlan`] mixes: the strip parts of every job (the
//! layer's Volume, Pan and rack) move off the job onto its layer's node, so
//! what stays on a clip is only what lies below the strip (fades, clip gain,
//! the layer's Gain, nested comps). Routing, solo and mute are worked out
//! here once, on nodes, rather than by copying jobs. Inside a nested comp
//! nothing changes: its buses, solo and racks still fold into the clips
//! that reach its Precomp strip.
//!
//! Built on the thread that asks (the bridge's prepare worker, an export),
//! never the callback. Plugins may be opened and racks baked, so it can take
//! a while, and it stops when asked to.

use lumit_audio::live::LiveBank;
use lumit_audio::mix::{db_to_gain, pan_gains, GainEnvelope, MASTER_CEILING};
use lumit_audio::plan::{
    Fader, MixNode, MixPlan, MixSend, PlacedClip, PlanRack, MAX_NODES, MAX_SENDS, NONE,
};
use lumit_audio::rack::{RackOpener, RackRing};
use lumit_audio::stream::Source;
use lumit_core::anim::Property;
use lumit_core::model::{Composition, Document, Layer};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use uuid::Uuid;

use super::bake::{moves, read};
use super::jobs::{AudioChain, AudioJob, Carrier, DrivenVolume, ExpressionScope};
use super::live::{place_clips, RackBakes};
use super::order::{edges_on, order, Edges};
use super::rack_stream::{sounds, NodeRack};

/// One node of a built plan, as the bridge needs to find it again.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    /// The layer it is: a sounding layer or a Bus layer of the comp, or a
    /// node graph's Read.
    pub layer: Uuid,
    /// A Bus layer rather than a strip.
    pub bus: bool,
    /// Its sends, in the layer's send order: the bus sent to and the send's
    /// slot in the plan's [`LiveBank`], None past [`MAX_SENDS`].
    pub sends: Vec<(Uuid, Option<u16>)>,
}

/// What each node of a built plan is, in the plan's node order. Node `i`'s
/// fader is slot `i` of the plan's [`LiveBank`], and the master's is slot
/// `nodes.len()`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphInfo {
    pub nodes: Vec<GraphNode>,
}

impl GraphInfo {
    /// The node of `layer`, a strip or a bus.
    #[must_use]
    pub fn node_of(&self, layer: Uuid) -> Option<u16> {
        self.nodes
            .iter()
            .position(|n| n.layer == layer)
            .and_then(|i| u16::try_from(i).ok())
    }
}

/// The plan a new one replaces, and what it was built from: its nodes carry
/// over and an unchanged rack keeps its ring (and so its stream and its
/// tail). Its held faders are seeded by whoever installs the new plan.
#[derive(Clone, Copy)]
pub struct GraphPrev<'a> {
    pub plan: &'a MixPlan,
    pub info: &'a GraphInfo,
}

/// A build stopped because a newer one was asked for. Nothing it made was
/// kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the mix was stopped for a newer one")]
pub struct Cancelled;

/// One node while it is worked out, before its place in the order is known.
struct Spec<'a> {
    layer: Uuid,
    /// None for a node graph's Read, which has no layer of the comp and
    /// keeps its jobs as they came.
    of: Option<&'a Layer>,
    bus: bool,
    has_jobs: bool,
    rack: Option<Arc<AudioChain>>,
    driven: Option<Arc<DrivenVolume>>,
    /// The stretch its jobs cover, in mixed seconds.
    span: (f64, f64),
}

/// **The comp's desk.** `jobs` are the comp's audio jobs as
/// [`crate::headless::AudioJobsBuilder::audio_jobs`] walked them; each sums
/// into the node of its strip ([`AudioJob::layer`]). `source_of` names each
/// job's sound at `rate`. `bakes` carries racked clips' sound from one build
/// to the next. `prev` is the plan this one replaces, if any. `offline` for
/// an export: exact lengths and the racks' slower path. `cancel` stops it
/// between jobs and between rack blocks, with [`Cancelled`]. `listen` is a
/// layer asked for by name: it stands as soloed, so it is heard whatever its
/// switches and any solo elsewhere say.
///
/// The held faders of the plan this replaces are not carried here: the
/// caller seeds them ([`MixPlan::seed_live`]) where it installs the plan, so
/// a control let go meanwhile is not lost.
///
/// The nodes are mixed in an order where every output, send and duck key
/// runs forward, buses after strips where nothing else decides, then in
/// layer order. A plan that fails its own check (it never should) comes back
/// with every clip straight to the master, so the comp is still heard.
#[allow(clippy::too_many_arguments)]
pub fn build_graph(
    doc: &Arc<Document>,
    comp: &Composition,
    mut jobs: Vec<AudioJob>,
    source_of: &dyn Fn(&AudioJob) -> Option<Arc<Source>>,
    rate: u32,
    duration_s: f64,
    bakes: &mut RackBakes,
    prev: Option<GraphPrev<'_>>,
    offline: bool,
    cancel: &AtomicBool,
    listen: Option<Uuid>,
) -> Result<(MixPlan, GraphInfo), Cancelled> {
    let total_frames = (duration_s * f64::from(rate)).round().max(0.0) as usize;
    let mut specs = specs(doc, comp, &jobs);
    let index: HashMap<Uuid, usize> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.layer, i))
        .collect();

    // The strip parts move from the jobs to their nodes.
    for job in &mut jobs {
        let Some(&s) = index.get(&job.layer) else {
            fold_gain(job, comp);
            continue;
        };
        take_strip(&mut specs[s], job);
    }

    // The edges, then the order.
    let mut edges: Vec<Edges> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| edges_of(i, s, &index, &specs, listen))
        .collect();
    let order = order(&mut edges);
    let mut at = vec![0usize; specs.len()];
    for (i, &s) in order.iter().enumerate() {
        at[s] = i;
    }
    let edges: Vec<Edges> = order
        .iter()
        .map(|&s| {
            let e = &edges[s];
            Edges {
                out: e.out.map(|o| at[o]),
                sends: e.sends.iter().map(|&w| at[w]).collect(),
                quiet: e.quiet.iter().map(|&w| at[w]).collect(),
                key: e.key.map(|k| at[k]),
                ..e.clone()
            }
        })
        .collect();
    let specs: Vec<Spec<'_>> = {
        let mut slots: Vec<Option<Spec<'_>>> = specs.into_iter().map(Some).collect();
        order.iter().filter_map(|&s| slots[s].take()).collect()
    };
    let on = edges_on(&edges);
    let keys: Vec<bool> = (0..edges.len())
        .map(|k| edges.iter().any(|e| e.key == Some(k)))
        .collect();

    let node_at: HashMap<Uuid, u16> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.layer, i as u16))
        .collect();
    let mut clips = place_clips(
        &jobs,
        source_of,
        &|layer| node_at.get(&layer).copied().unwrap_or(NONE),
        rate,
        offline,
        bakes,
        cancel,
    )?;
    // A node with nothing on is not heard, so neither are its clips, unless
    // another node is lowered behind it.
    for clip in &mut clips {
        let n = usize::from(clip.node);
        if let Some((out, sends)) = on.get(n) {
            clip.silent = !*out && !sends.iter().any(|s| *s) && !keys[n];
        }
    }

    // The nodes.
    let mut info = GraphInfo::default();
    let mut nodes = Vec::with_capacity(specs.len());
    let mut slots = 0usize;
    let mut whole: Vec<(usize, Arc<AudioChain>)> = Vec::new();
    let mut racks: Vec<PlanRack> = Vec::new();
    for (i, (spec, e)) in specs.iter().zip(&edges).enumerate() {
        let scope = spec
            .of
            .and_then(|l| ExpressionScope::of(doc, comp.id, l.id, 0.0));
        let mut fader = match spec.of {
            Some(l) => fader_of(
                &l.volume_db,
                &l.pan,
                l.start_offset.0.to_f64(),
                scope.as_deref(),
                spec.driven.as_deref().map(|d| (d, spec.span)),
                total_frames,
                rate,
            ),
            None => Fader::default(),
        };
        fader.live = Some(i as u16);
        let mut graph_node = GraphNode {
            layer: spec.layer,
            bus: spec.bus,
            sends: Vec::new(),
        };
        let mut sends = Vec::new();
        if let Some(l) = spec.of {
            // Matched by the bus sent to, which a layer sends to once: a
            // send the order had to drop is not here.
            let wanted = l.audio_routes.sends.iter().filter_map(|send| {
                let to = usize::from(node_at.get(&send.bus).copied()?);
                let k = e.sends.iter().position(|&w| w == to)?;
                Some((send, to, on[i].1.get(k).copied().unwrap_or(false)))
            });
            for (send, to, send_on) in wanted {
                let live = (slots < MAX_SENDS).then_some(slots as u16);
                slots += 1;
                let mut level = fader_of(
                    &send.level_db,
                    &Property::zero(),
                    l.start_offset.0.to_f64(),
                    scope.as_deref(),
                    None,
                    total_frames,
                    rate,
                );
                level.live = live;
                graph_node.sends.push((send.bus, live));
                sends.push(MixSend {
                    to: to as u16,
                    level,
                    pre: send.pre,
                    on: send_on,
                });
            }
        }
        let duck = spec.of.and_then(|l| l.duck).zip(e.key).map(|(d, k)| {
            let frames = |ms: f64| ((ms * f64::from(rate) / 1000.0).round().max(1.0)) as u32;
            lumit_audio::duck::Duck {
                key: k as u16,
                depth: db_to_gain(d.depth_db),
                threshold: db_to_gain(d.threshold_db),
                attack_frames: frames(d.attack_ms),
                hold_frames: frames(d.hold_ms),
                release_frames: frames(d.release_ms),
            }
        });
        let mut node = MixNode {
            fader,
            out: e.out.map(|o| o as u16),
            out_on: on[i].0,
            sends,
            rack: None,
            duck,
        };
        // A rack nobody hears is not run.
        let heard = on[i].0 || on[i].1.iter().any(|s| *s) || keys[i];
        if let Some(chain) = spec.rack.as_ref().filter(|_| heard) {
            // A rack that has to hear its whole input is baked once the plan
            // is checked; until then its rack stands, so mixing its input
            // mixes only what feeds it.
            let opener = NodeRack::new(Arc::clone(chain), bakes.stamp);
            if opener.bakes_whole(rate) {
                whole.push((i, Arc::clone(chain)));
            }
            node.rack = Some(racks.len() as u16);
            racks.push(PlanRack {
                node: i as u16,
                ring: ring_for(prev, spec, opener.identity(), rate),
                opener: Arc::new(opener),
            });
        }
        nodes.push(node);
        info.nodes.push(graph_node);
    }

    // The master.
    let master_keys = comp.master_keys.clone().map(|animation| Property {
        animation,
        extra: serde_json::Map::new(),
    });
    let mut master = match &master_keys {
        Some(keys) => fader_of(keys, &Property::zero(), 0.0, None, None, total_frames, rate),
        None => Fader::constant([db_to_gain(comp.master_volume_db); 2]),
    };
    master.live = Some(nodes.len() as u16);
    let ceiling = comp
        .loudness_target
        .map_or(MASTER_CEILING, |t| db_to_gain(t.true_peak_db));

    let carry = info
        .nodes
        .iter()
        .map(|n| {
            prev.and_then(|p| {
                p.info
                    .nodes
                    .iter()
                    .position(|o| o.layer == n.layer && o.bus == n.bus)
            })
            .map_or(NONE, |o| o as u16)
        })
        .collect();
    let fallback = clips.clone();
    let mut plan = MixPlan::default();
    plan.live = Arc::new(LiveBank::new(nodes.len(), slots.min(MAX_SENDS)));
    plan.clips = clips;
    plan.nodes = nodes;
    plan.total_frames = total_frames;
    plan.master = master;
    plan.ceiling = ceiling;
    plan.racks = racks;
    plan.carry = carry;
    let plan = match plan.finish() {
        Ok(plan) => super::whole::bake_all(plan, whole, rate, offline, bakes, cancel)?,
        Err(_) => unrouted(fallback, total_frames),
    };
    Ok((plan, info))
}

/// The nodes of the comp being mixed, in layer order: each Bus layer, each
/// layer with jobs, each muted layer that would sound (no clips, so its meter
/// reads nought and its fader still answers), then any node graph Read the
/// jobs name. Past [`MAX_NODES`] the last strips have no node: their jobs keep
/// their strip parts and go straight to the master.
fn specs<'a>(doc: &Arc<Document>, comp: &'a Composition, jobs: &[AudioJob]) -> Vec<Spec<'a>> {
    let with_jobs: std::collections::HashSet<Uuid> = jobs.iter().map(|j| j.layer).collect();
    let mut sounding = crate::headless::AudioJobsBuilder::new();
    let mut specs: Vec<Spec<'a>> = Vec::new();
    for l in &comp.layers {
        let has_jobs = with_jobs.contains(&l.id);
        let strip =
            !l.is_bus() && (has_jobs || (!l.switches.audible && sounding.layer_has_audio(doc, l)));
        if !l.is_bus() && !strip {
            continue;
        }
        let rack = if l.is_bus() {
            crate::headless::bus_chain_of(doc, comp, l, l.start_offset.0.to_f64(), 0.0)
        } else {
            None
        };
        specs.push(Spec {
            layer: l.id,
            of: Some(l),
            bus: l.is_bus(),
            has_jobs,
            rack,
            driven: None,
            span: (f64::INFINITY, f64::NEG_INFINITY),
        });
    }
    let mut reads: Vec<Uuid> = Vec::new();
    for job in jobs {
        if !comp.layers.iter().any(|l| l.id == job.layer) && !reads.contains(&job.layer) {
            reads.push(job.layer);
        }
    }
    specs.extend(reads.into_iter().map(|layer| Spec {
        layer,
        of: None,
        bus: false,
        has_jobs: true,
        rack: None,
        driven: None,
        span: (f64::INFINITY, f64::NEG_INFINITY),
    }));
    let buses = specs.iter().filter(|s| s.bus).count().min(MAX_NODES);
    let mut strips = 0usize;
    let mut kept_buses = 0usize;
    specs.retain(|s| {
        if s.bus {
            kept_buses += 1;
            kept_buses <= MAX_NODES
        } else {
            strips += 1;
            strips + buses <= MAX_NODES
        }
    });
    specs
}

/// Move `job`'s strip parts onto its node: for a job of its own, its Volume,
/// Pan, driven Volume and rack; for one arriving through a Precomp layer or a
/// composition clip, that carrier's Volume, Pan and rack. What is left on the
/// job is what lies below the strip, with the layer's Gain in the Volume's
/// place. A node graph's Read keeps its job as it is.
fn take_strip(spec: &mut Spec<'_>, job: &mut AudioJob) {
    spec.span = (spec.span.0.min(job.in_s), spec.span.1.max(job.out_s));
    let Some(layer) = spec.of else {
        return;
    };
    let gain = Property::fixed(layer.audio_gain_db);
    match job.carriers.first_mut() {
        Some(strip) => {
            if spec.rack.is_none() {
                spec.rack = strip.chain.take().filter(|c| sounds(&c.effects));
            }
            strip.chain = None;
            strip.volume = gain;
            strip.pan = Property::zero();
        }
        None => {
            if spec.rack.is_none() {
                spec.rack = job.chain.clone().filter(|c| sounds(&c.effects));
            }
            if spec.driven.is_none() {
                spec.driven = job.driven.clone();
            }
            // A stack of picture effects is no rack, and leaving it on the
            // job costs nothing: it opens nothing as audio.
            job.chain = None;
            job.driven = None;
            job.volume = gain;
            job.pan = Property::zero();
        }
    }
}

/// A job whose strip has no node keeps its strip parts and goes straight to
/// the master; its layer's Gain rides in as one more gain below the strip.
fn fold_gain(job: &mut AudioJob, comp: &Composition) {
    let gain_db = comp
        .layers
        .iter()
        .find(|l| l.id == job.layer)
        .map_or(0.0, |l| l.audio_gain_db);
    if gain_db != 0.0 {
        job.carriers.push(Carrier {
            volume: Property::fixed(gain_db),
            pan: Property::zero(),
            offset_s: 0.0,
            fade: None,
            chain: None,
            retime: None,
            scope: None,
        });
    }
}

/// A spec's edges and what the solo rule reads, by spec index.
fn edges_of(
    me: usize,
    spec: &Spec<'_>,
    index: &HashMap<Uuid, usize>,
    specs: &[Spec<'_>],
    listen: Option<Uuid>,
) -> Edges {
    let bus = |id: Uuid| index.get(&id).copied().filter(|&b| specs[b].bus && b != me);
    // The specs are in layer order already.
    let rank = (spec.bus, me);
    let Some(l) = spec.of else {
        return Edges {
            marked: false,
            rank,
            ..Edges::default()
        };
    };
    let routes = &l.audio_routes;
    // A layer heard by name (its jobs are here though its switch is off)
    // stands as soloed, so a solo elsewhere does not silence it.
    let named = spec.has_jobs && !l.switches.audible;
    let heard = listen == Some(l.id);
    Edges {
        out: routes.output.and_then(bus),
        sends: routes.sends.iter().filter_map(|s| bus(s.bus)).collect(),
        quiet: routes
            .sends
            .iter()
            .filter(|s| s.muted)
            .filter_map(|s| bus(s.bus))
            .collect(),
        key: l
            .duck
            .and_then(|d| index.get(&d.behind).copied())
            .filter(|&k| k != me),
        marked: l.switches.solo || l.switches.solo_safe || named || heard,
        solo: l.switches.solo || heard,
        muted: !l.switches.audible && !spec.has_jobs,
        rank,
    }
}

/// A fader from a Volume (dB) and a Pan read in layer time (`offset_s` is
/// where layer time 0 sits), a constant when neither moves, else an envelope
/// at 10 ms on timeline frames from 0. A driven Volume answers its chain over
/// `span` and its keys outside it.
fn fader_of(
    volume: &Property,
    pan: &Property,
    offset_s: f64,
    scope: Option<&ExpressionScope>,
    driven: Option<(&DrivenVolume, (f64, f64))>,
    total_frames: usize,
    rate: u32,
) -> Fader {
    let at = |t: f64| {
        let db = driven
            .filter(|(_, span)| t >= span.0 && t <= span.1)
            .and_then(|(d, _)| d.db_at(t))
            .unwrap_or_else(|| read(volume, t - offset_s, t, scope));
        let g = db_to_gain(db);
        let p = pan_gains(read(pan, t - offset_s, t, scope));
        [p[0] * g, p[1] * g]
    };
    if !(moves(volume) || moves(pan) || driven.is_some()) {
        return Fader::constant(at(0.0));
    }
    let stride = (rate / 100).max(1);
    let points = (0..total_frames / stride as usize + 2)
        .map(|p| at((p * stride as usize) as f64 / f64::from(rate)))
        .collect();
    Fader {
        gain: [1.0, 1.0],
        envelope: Some(Arc::new(GainEnvelope { stride, points })),
        live: None,
    }
}

/// The ring for a node's rack: the one its node had in the plan before when
/// the rack is unchanged, so its stream runs on, else a new one.
fn ring_for(
    prev: Option<GraphPrev<'_>>,
    spec: &Spec<'_>,
    identity: u64,
    rate: u32,
) -> Arc<RackRing> {
    prev.and_then(|p| {
        let o = p
            .info
            .nodes
            .iter()
            .position(|n| n.layer == spec.layer && n.bus == spec.bus)?;
        let rack = p.plan.racks.get(usize::from(p.plan.nodes.get(o)?.rack?))?;
        (rack.opener.identity() == identity).then(|| Arc::clone(&rack.ring))
    })
    .unwrap_or_else(|| Arc::new(RackRing::new(rate)))
}

/// What plays if a built plan fails its own check, which it never should:
/// every clip summed at unity straight out, so the comp is heard with its
/// strips' faders left out rather than not at all.
pub(super) fn unrouted(mut clips: Vec<PlacedClip>, total_frames: usize) -> MixPlan {
    for clip in &mut clips {
        clip.silent = false;
    }
    MixPlan::single(clips, total_frames)
}
