//! Buses and sends of a nested comp, in the audio jobs walk. At the comp
//! being mixed they are the desk's ([`crate::mix::graph`]), unless the walk
//! is asked to fold them in.
//!
//! A Bus layer makes no sound of its own. What is routed to it arrives as
//! jobs carrying one more [`Carrier`]: the bus's Volume and Pan, and its rack.
//! The bus stage that already sums a Precomp layer's sound for its rack
//! ([`crate::mix::stage::bus_runs`]) then sums the bus too, so playback and export
//! hear it the same way. A send is a copy of the layer's jobs with its level as
//! one more carrier inside the bus's.

use crate::mix::jobs::{AudioChain, AudioJob, Carrier, ExpressionScope};
use lumit_core::model::{Composition, Document, Layer};
use std::sync::Arc;
use uuid::Uuid;

/// One Bus layer of the comp being walked, as its jobs need it.
pub(super) struct Bus {
    id: Uuid,
    carrier: Carrier,
    heard: bool,
    solo: bool,
}

/// The comp's buses. A bus passes sound for the whole comp, whatever its in
/// and out points. A bus with its audible switch off still routes, and
/// silences what goes into it.
pub(super) fn buses(doc: &Arc<Document>, comp: &Composition, base_s: f64) -> Vec<Bus> {
    comp.layers
        .iter()
        .filter(|l| l.is_bus())
        .map(|l| {
            let offset_s = l.start_offset.0.to_f64() + base_s;
            Bus {
                id: l.id,
                carrier: Carrier {
                    volume: l.volume_db.clone(),
                    pan: l.pan.clone(),
                    offset_s,
                    fade: None,
                    chain: super::bus_chain_of(doc, comp, l, offset_s, base_s),
                    retime: None,
                    scope: crate::mix::jobs::ExpressionScope::of(doc, comp.id, l.id, base_s),
                },
                heard: l.switches.audible,
                solo: l.switches.solo,
            }
        })
        .collect()
}

/// Whether `layer` feeds a soloed bus. Soloing a bus solos what goes into it.
pub(super) fn feeds_solo(buses: &[Bus], layer: &Layer) -> bool {
    let routes = &layer.audio_routes;
    buses.iter().filter(|b| b.solo).any(|b| {
        routes.output == Some(b.id) || routes.sends.iter().any(|s| s.bus == b.id && !s.muted)
    })
}

/// Route the jobs `layer` has just added (`jobs[from..]`) to its buses.
///
/// `depth` is how many carriers the comp's own jobs arrive with: everything a
/// layer adds goes in there, ahead of the layer's own Precomp carrier. `top`
/// is the comp being mixed, where a send meters on the bus's own row.
/// `base_s` is where the comp's time 0 sits on the mixed timeline.
pub(super) fn route(
    buses: &[Bus],
    layer: &Layer,
    base_s: f64,
    depth: usize,
    top: bool,
    from: usize,
    jobs: &mut Vec<AudioJob>,
) {
    let routes = &layer.audio_routes;
    if routes.is_master() || layer.is_bus() || from >= jobs.len() {
        return;
    }
    let bus = |id: Uuid| buses.iter().find(|b| b.id == id);
    let offset_s = layer.start_offset.0.to_f64() + base_s;
    // The layer's own sound stays where it was in the list, and its sends
    // follow it.
    let own: Vec<AudioJob> = jobs.drain(from..).collect();
    match routes.output.and_then(bus) {
        Some(bus) if bus.heard => {
            for job in &own {
                let mut job = job.clone();
                job.carriers.insert(depth, bus.carrier.clone());
                // A rack sums what it is given into one run, and that run
                // meters on the bus's own row.
                if top && bus.carrier.chain.is_some() {
                    job.layer = bus.id;
                }
                jobs.push(job);
            }
        }
        // Into a bus that is switched off: nothing.
        Some(_) => {}
        // To a layer that is gone or is not a bus: the master.
        None => jobs.extend(own.iter().cloned()),
    }
    for send in routes.sends.iter().filter(|s| !s.muted) {
        let Some(bus) = bus(send.bus).filter(|b| b.heard) else {
            continue;
        };
        for job in &own {
            let mut job = fresh(job, depth);
            if send.pre {
                before_fader(&mut job, depth);
            }
            let level = Carrier {
                volume: send.level_db.clone(),
                pan: lumit_core::anim::Property::zero(),
                // Keyed in layer time, as the layer's own Volume is.
                offset_s,
                fade: None,
                chain: None,
                retime: None,
                // The send's level is the sending layer's row, so an
                // expression on it reads that layer.
                scope: bus.carrier.scope.as_ref().map(|s| {
                    Arc::new(ExpressionScope {
                        layer: layer.id,
                        ..(**s).clone()
                    })
                }),
            };
            job.carriers
                .splice(depth..depth, [bus.carrier.clone(), level]);
            if top {
                job.layer = bus.id;
            }
            jobs.push(job);
        }
    }
}

/// A copy of `job` whose racks and retimes below `depth` are its own. The bus
/// stage takes carriers holding the same rack for one bus, so a send that
/// shared them with the layer's own sound would be summed in with it.
fn fresh(job: &AudioJob, depth: usize) -> AudioJob {
    let mut job = job.clone();
    for c in job.carriers.iter_mut().skip(depth) {
        if let Some(chain) = &c.chain {
            c.chain = Some(Arc::new(AudioChain {
                doc: Arc::clone(&chain.doc),
                comp: chain.comp,
                layer: chain.layer,
                effects: chain.effects.clone(),
                graph: chain.graph.clone(),
                offset_s: chain.offset_s,
                base_s: chain.base_s,
            }));
        }
        if let Some(retime) = &c.retime {
            c.retime = Some(Arc::new((**retime).clone()));
        }
    }
    job
}

/// Take the layer's Volume, Pan and duck out of a send. They sit on the job
/// itself, or on the layer's own carrier where the sound comes through a
/// nested comp. A clip's fades and level stay: they are the clip's.
fn before_fader(job: &mut AudioJob, depth: usize) {
    match job.carriers.get_mut(depth) {
        Some(own) => {
            own.volume = lumit_core::anim::Property::zero();
            own.pan = lumit_core::anim::Property::zero();
        }
        None => {
            job.volume = lumit_core::anim::Property::zero();
            job.pan = lumit_core::anim::Property::zero();
            job.driven = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::tests::{push_comp, push_footage_item, push_layer, seed_has_audio};
    use super::super::AudioJobsBuilder;
    use crate::mix::graph::build_graph;
    use crate::mix::live::RackBakes;
    use lumit_core::anim::Property;
    use lumit_core::model::{Document, LayerKind, ProjectItem};
    use lumit_core::routes::{AudioRoutes, AudioSend};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use uuid::Uuid;

    /// A comp of `layers` footage layers and `buses` Bus layers after them,
    /// every footage layer playing the same file.
    fn desk(layers: usize, buses: usize) -> (Document, Uuid, Vec<Uuid>) {
        let mut doc = Document::new();
        let song = push_footage_item(&mut doc, "song.wav");
        seed_has_audio(song);
        let comp = push_comp(&mut doc, "mix", 32, 32);
        for _ in 0..layers {
            push_layer(&mut doc, comp, LayerKind::Footage { item: song });
        }
        for _ in 0..buses {
            push_layer(&mut doc, comp, LayerKind::Null);
        }
        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        for l in &mut c.layers[layers..] {
            l.audio_only = true;
        }
        let ids = c.layers.iter().map(|l| l.id).collect();
        (doc, comp, ids)
    }

    /// The level the master hears a second of steady 0.1 at, half way in,
    /// with every layer playing it.
    fn heard(doc: &Document, comp: Uuid) -> f32 {
        let doc = Arc::new(doc.clone());
        let c = doc.comp(comp).unwrap().clone();
        let jobs = AudioJobsBuilder::new().audio_jobs(&doc, &c);
        let level = lumit_audio::stream::Source::whole(Arc::new(lumit_media::audio::AudioBuffer {
            rate: 48_000,
            samples: vec![0.1; 96_000],
        }));
        let (plan, _) = build_graph(
            &doc,
            &c,
            jobs,
            &|_| Some(Arc::clone(&level)),
            48_000,
            1.0,
            &mut RackBakes::default(),
            None,
            true,
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        plan.frame_at(24_000).0
    }

    fn db(db: f64) -> f32 {
        lumit_audio::mix::db_to_gain(db)
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    /// One layer goes out through a bus, another stays on the master and sends
    /// to it from before its Volume. Each reaches the master at the gain the
    /// routing says, and a bus switched off silences what goes into it.
    #[test]
    fn output_and_sends_reach_the_master_through_the_bus() {
        let (mut doc, comp, ids) = desk(2, 1);
        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        let bus = ids[2];
        c.layers[2].volume_db = Property::fixed(-6.0);
        c.layers[0].audio_routes.output = Some(bus);
        c.layers[1].volume_db = Property::fixed(-20.0);
        c.layers[1].audio_routes = AudioRoutes {
            output: None,
            sends: vec![AudioSend {
                bus,
                level_db: Property::fixed(-6.0),
                pre: true,
                muted: false,
            }],
        };
        // Out through the bus at -6, the dry sound at -20, and the send
        // skipping the -20 for -6 and the bus's -6.
        let want = 0.1 * (db(-6.0) + db(-20.0) + db(-12.0));
        let got = heard(&doc, comp);
        assert!(near(got, want), "{got} for {want}");

        // The send muted sends nothing, and the rest is as it was.
        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        c.layers[1].audio_routes.sends[0].muted = true;
        let got = heard(&doc, comp);
        let want = 0.1 * (db(-6.0) + db(-20.0));
        assert!(near(got, want), "a muted send: {got} for {want}");
        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        c.layers[1].audio_routes.sends[0].muted = false;

        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        c.layers[2].switches.audible = false;
        let got = heard(&doc, comp);
        assert!(
            near(got, 0.1 * db(-20.0)),
            "only the dry sound is left: {got}"
        );
    }

    /// **A bus feeds a bus, and a solo keeps the paths it needs.** A layer
    /// goes through one bus into another and on to the master, at each
    /// fader's gain. Soloing it, and a layer that sends to a third bus, keeps
    /// both of their buses heard and silences a layer that only goes
    /// straight to the master. That third bus solo safe keeps its own way
    /// out, but lets in nothing that is not soloed.
    #[test]
    fn a_bus_feeds_a_bus_and_a_solo_keeps_its_buses() {
        let (mut doc, comp, ids) = desk(3, 3);
        let (x, z, y) = (ids[3], ids[4], ids[5]);
        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        // Layer 0 through X into Z, every fader at -6.
        c.layers[0].volume_db = Property::fixed(-6.0);
        c.layers[0].audio_routes.output = Some(x);
        c.layers[3].volume_db = Property::fixed(-6.0);
        c.layers[3].audio_routes.output = Some(z);
        c.layers[4].volume_db = Property::fixed(-6.0);
        // Layer 1 straight out; layer 2 straight out and sending to Y at -12.
        c.layers[2].audio_routes.sends = vec![AudioSend {
            bus: y,
            level_db: Property::fixed(-12.0),
            pre: false,
            muted: false,
        }];
        let got = heard(&doc, comp);
        let want = 0.1 * (db(-18.0) + 1.0 + 1.0 + db(-12.0));
        assert!(near(got, want), "everything: {got} for {want}");

        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        c.layers[0].switches.solo = true;
        c.layers[2].switches.solo = true;
        let got = heard(&doc, comp);
        let want = 0.1 * (db(-18.0) + 1.0 + db(-12.0));
        assert!(
            near(got, want),
            "the soloed two and their buses: {got} for {want}"
        );

        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        c.layers[2].switches.solo = false;
        c.layers[5].switches.solo_safe = true;
        let got = heard(&doc, comp);
        let want = 0.1 * db(-18.0);
        assert!(
            near(got, want),
            "nothing sent into the safe bus: {got} for {want}"
        );
    }
}
