//! Buses and sends in the audio jobs walk.
//!
//! A Bus layer makes no sound of its own. What is routed to it arrives as
//! jobs carrying one more [`Carrier`]: the bus's Volume and Pan, and its rack.
//! The bus stage that already sums a Precomp layer's sound for its rack
//! ([`crate::mix::stage::bus_runs`]) then sums the bus too, so playback and export
//! hear it the same way. A send is a copy of the layer's jobs with its level as
//! one more carrier inside the bus's.

use crate::mix::jobs::{AudioChain, AudioJob, Carrier};
use lumit_core::model::{Composition, Document, Layer};
use std::sync::Arc;
use uuid::Uuid;

/// One Bus layer of the comp being walked, as its jobs need it.
pub(super) struct Bus {
    id: Uuid,
    carrier: Carrier,
    /// The bus's span on the mixed timeline: it passes nothing outside it.
    span: (f64, f64),
    heard: bool,
    solo: bool,
}

/// The comp's buses. A bus with its audible switch off still routes, and
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
                },
                span: (
                    l.in_point.0.to_f64() + base_s,
                    l.out_point.0.to_f64() + base_s,
                ),
                heard: l.switches.audible,
                solo: l.switches.solo,
            }
        })
        .collect()
}

/// Whether `layer` feeds a soloed bus. Soloing a bus solos what goes into it.
pub(super) fn feeds_solo(buses: &[Bus], layer: &Layer) -> bool {
    let routes = &layer.audio_routes;
    buses
        .iter()
        .filter(|b| b.solo)
        .any(|b| routes.output == Some(b.id) || routes.sends.iter().any(|s| s.bus == b.id))
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
                jobs.extend(within(job, bus.span));
            }
        }
        // Into a bus that is switched off: nothing.
        Some(_) => {}
        // To a layer that is gone or is not a bus: the master.
        None => jobs.extend(own.iter().cloned()),
    }
    for send in &routes.sends {
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
            };
            job.carriers
                .splice(depth..depth, [bus.carrier.clone(), level]);
            if top {
                job.layer = bus.id;
            }
            if let Some(job) = within(job, bus.span) {
                jobs.push(job);
            }
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

/// `job` cut to `span`, or `None` where nothing of it is left.
fn within(mut job: AudioJob, span: (f64, f64)) -> Option<AudioJob> {
    job.in_s = job.in_s.max(span.0);
    job.out_s = job.out_s.min(span.1);
    (job.out_s > job.in_s).then_some(job)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::tests::{push_comp, push_footage_item, push_layer, seed_has_audio};
    use super::super::AudioJobsBuilder;
    use crate::mix::stage::{bus_runs, PlacedJob};
    use lumit_core::anim::Property;
    use lumit_core::model::{LayerKind, ProjectItem};
    use lumit_core::routes::{AudioRoutes, AudioSend};
    use std::sync::Arc;

    /// One layer goes out through a bus, another stays on the master and sends
    /// to it from before its Volume. Each reaches the master at the gain the
    /// routing says, and a bus switched off silences what goes into it.
    #[test]
    fn output_and_sends_reach_the_master_through_the_bus() {
        let mut doc = lumit_core::model::Document::new();
        let song = push_footage_item(&mut doc, "song.wav");
        seed_has_audio(song);
        let comp = push_comp(&mut doc, "mix", 32, 32);
        push_layer(&mut doc, comp, LayerKind::Footage { item: song });
        push_layer(&mut doc, comp, LayerKind::Footage { item: song });
        push_layer(&mut doc, comp, LayerKind::Null);
        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        let bus = c.layers[2].id;
        c.layers[2].audio_only = true;
        c.layers[2].volume_db = Property::fixed(-6.0);
        c.layers[0].audio_routes.output = Some(bus);
        c.layers[1].volume_db = Property::fixed(-20.0);
        c.layers[1].audio_routes = AudioRoutes {
            output: None,
            sends: vec![AudioSend {
                bus,
                level_db: Property::fixed(-6.0),
                pre: true,
            }],
        };

        let mix = |doc: &lumit_core::model::Document| {
            let c = doc.comp(comp).unwrap().clone();
            let jobs = AudioJobsBuilder::new().audio_jobs(&Arc::new(doc.clone()), &c);
            let samples = [0.0f32; 2];
            let placed: Vec<PlacedJob<'_>> = jobs
                .iter()
                .map(|job| PlacedJob {
                    job,
                    start_frame: 0,
                    samples: &samples,
                })
                .collect();
            let runs = bus_runs(&placed, 48_000, true);
            runs.iter()
                .map(|r| (r.layer, r.gain[0]))
                .collect::<Vec<_>>()
        };
        let db = |db: f64| lumit_audio::mix::db_to_gain(db);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        let c = doc.comp(comp).unwrap();
        let (a, b) = (c.layers[0].id, c.layers[1].id);

        let runs = mix(&doc);
        assert_eq!(runs.len(), 3);
        assert!(
            runs[0].0 == a && near(runs[0].1, db(-6.0)),
            "out through the bus"
        );
        assert!(
            runs[1].0 == b && near(runs[1].1, db(-20.0)),
            "the dry sound"
        );
        assert!(
            runs[2].0 == bus && near(runs[2].1, db(-12.0)),
            "the send skips the -20 and meters on the bus"
        );

        let ProjectItem::Composition(c) = doc.item_mut(comp).unwrap() else {
            unreachable!()
        };
        c.layers[2].switches.audible = false;
        let runs = mix(&doc);
        assert_eq!(runs.len(), 1, "only the dry sound is left");
        assert_eq!(runs[0].0, b);
    }
}
