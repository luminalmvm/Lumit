//! One comp shown at two times in one frame draws two pictures, by every road
//! a comp is reached. The footage's pixels are coloured by their source frame,
//! so a draw says which frame it holds.

use std::collections::HashMap;
use std::sync::Arc;

use lumit_core::anim::Property;
use lumit_core::comp_graph::{CompGraph, GraphEdge, GraphNode};
use lumit_core::model::{Composition, Document, Layer, LayerKind, ProjectItem};
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::time::{CompTime, Rational};
use uuid::Uuid;

use super::render_below_at_tests::{
    clip, comp_with, posterize_adjustment, previous_frame_effect, text_layer,
};
use crate::decode::{CompJob, CompLayerPixels};
use crate::draw::{CompLayerDraw, DrawSource, GraphStep};
use crate::plan::{plan_comp_frame, Quality};

type Probes = HashMap<Uuid, crate::source::SourceProbe>;

fn flat(job: &CompJob, frame: usize) -> CompLayerPixels {
    CompLayerPixels {
        layer: job.layer,
        at: job.at,
        elsewhere: Vec::new(),
        width: 8,
        height: 8,
        rgba: [frame as u8, 0, 0, 255].repeat(64).into(),
        format: lumit_media::decode::PixelFormat::Srgb8,
        natural_w: 320,
        natural_h: 180,
        temporal: Vec::new(),
        flow_fields: Vec::new(),
        shutter: Vec::new(),
        source_key: frame as u128,
        source_frame: frame as i64,
    }
}

fn decoded(job: &CompJob) -> CompLayerPixels {
    let mut lp = flat(job, job.source_frame);
    lp.shutter = job
        .shutter
        .iter()
        .map(|s| (s.offset, Some(Box::new(flat(job, s.source_frame)))))
        .collect();
    lp
}

// A frame planned, decoded and built, with the plan to look at.
fn built(
    doc: &Document,
    comp: &Composition,
    t: f64,
    probes: &Probes,
) -> (usize, Vec<CompLayerDraw>) {
    let jobs = plan_comp_frame(doc, comp, t, Quality::default(), probes);
    let layers = crate::placed::fold(jobs.iter().map(decoded).collect());
    let pixels: HashMap<Uuid, &CompLayerPixels> = layers.iter().map(|lp| (lp.layer, lp)).collect();
    let mut visited = vec![comp.id];
    let draws = super::build_comp_draws(&Arc::new(doc.clone()), comp, t, &pixels, &mut visited);
    (jobs.len(), draws)
}

// The source frame a draw shows.
fn shows(draw: &CompLayerDraw) -> u8 {
    match &draw.source {
        DrawSource::Pixels { rgba, .. } => rgba[0],
        DrawSource::Nested { draws, .. } => shows(draws.first().expect("the comp draws")),
        DrawSource::Graph(plan) => read_of(plan),
        _ => panic!("a draw of footage or a comp"),
    }
}

fn read_of(plan: &crate::draw::GraphDraw) -> u8 {
    match plan.steps.first() {
        Some(GraphStep::Read(read)) => shows(read),
        _ => panic!("the graph reads footage"),
    }
}

fn of(draws: &[CompLayerDraw], layer: Uuid) -> &CompLayerDraw {
    draws.iter().find(|d| d.layer == layer).expect("drawn")
}

fn starting(layer: &mut Layer, start: i64) {
    layer.start_offset = CompTime(Rational::new(start, 1).unwrap());
}

// A layer of comp `nested`, showing it `ahead` seconds on from the frame.
fn placed(nested: Uuid, ahead: i64) -> Layer {
    let mut l = text_layer(0.0);
    l.kind = LayerKind::Precomp { comp: nested };
    starting(&mut l, -ahead);
    l
}

#[test]
fn a_comp_shown_at_two_times_draws_each_from_its_own_frame() {
    let mut doc = Document::new();
    let (footage, probes) = clip(&mut doc);
    let LayerKind::Footage { item } = footage.kind else {
        panic!("a footage layer");
    };
    let nested = comp_with(10, vec![footage]);
    let id = nested.id;
    doc.items.push(ProjectItem::Composition(nested));

    // Once decodes once, and twice at one time is one picture.
    let one = placed(id, 0);
    let (jobs, _) = built(&doc, &comp_with(10, vec![one]), 1.0, &probes);
    assert_eq!(jobs, 1, "one layer, one decode");
    let same = [placed(id, 2), placed(id, 2)];
    let (ids, comp) = ([same[0].id, same[1].id], comp_with(10, same.to_vec()));
    let (jobs, draws) = built(&doc, &comp, 1.0, &probes);
    assert_eq!(jobs, 1, "one time, one decode");
    assert_eq!(ids.map(|l| shows(of(&draws, l))), [30, 30]);

    // By start time and by Retime, and with the frame order either way round.
    let (a, b, mut c) = (placed(id, 0), placed(id, 2), placed(id, 0));
    c.retime = Some(Property::fixed(5.0));
    let ids = [a.id, b.id, c.id];
    for layers in [
        vec![a.clone(), b.clone(), c.clone()],
        vec![c.clone(), b.clone(), a.clone()],
    ] {
        let (jobs, draws) = built(&doc, &comp_with(10, layers), 1.0, &probes);
        assert_eq!(jobs, 3, "one decode for each picture");
        assert_eq!(ids.map(|l| shows(of(&draws, l))), [10, 30, 50]);
    }

    // A clip on a Sequence layer, next to a Precomp layer of the same comp.
    let at = |s| Rational::new(s, 1).unwrap();
    let mut seq = text_layer(0.0);
    seq.kind = LayerKind::Sequence {
        clips: vec![Clip::new(ClipSource::Comp(id), at(4), at(6), at(0), at(2))],
    };
    let ids = [a.id, seq.id];
    let (_, draws) = built(&doc, &comp_with(10, vec![a.clone(), seq]), 1.0, &probes);
    assert_eq!(ids.map(|l| shows(of(&draws, l))), [10, 50]);

    // A hidden Precomp read as the matte of another of the same comp.
    let mut source = placed(id, 2);
    source.switches.visible = false;
    let mut matted = a.clone();
    matted.matte = Some(lumit_core::model::MatteRef {
        layer: source.id,
        channel: lumit_core::model::MatteChannel::Alpha,
        inverted: false,
        source: Default::default(),
    });
    let (_, draws) = built(&doc, &comp_with(10, vec![matted, source]), 1.0, &probes);
    let d = of(&draws, a.id);
    let inner = d
        .matte
        .as_ref()
        .and_then(|m| m.nested.as_ref())
        .expect("a comp matte");
    assert_eq!((shows(d), shows(&inner.draws[0])), (10, 30));

    // Held by a Posterize Time above both: each holds its own frame.
    let post = posterize_adjustment(1.0);
    let post_id = post.id;
    let comp = comp_with(10, vec![post, a.clone(), b.clone()]);
    let (_, draws) = built(&doc, &comp, 1.5, &probes);
    let held = of(&draws, post_id).temporal_below.as_ref().expect("held");
    assert_eq!([a.id, b.id].map(|l| shows(of(&held.draws, l))), [10, 30]);

    // The moments an effect reads are the ones of its own layer's frame.
    let mut reader = b.clone();
    reader.effects = vec![previous_frame_effect()];
    let (_, draws) = built(&doc, &comp_with(10, vec![a.clone(), reader]), 1.0, &probes);
    let d = of(&draws, b.id);
    let neighbours: Vec<u8> = d.flow_below.iter().map(|n| shows(&n.1[0])).collect();
    assert_eq!((shows(d), neighbours), (30, vec![29, 31]));
    assert_eq!(shows(of(&draws, a.id)), 10);

    // A node graph placed twice, and one applied by two layers.
    let (read, out) = (Uuid::now_v7(), Uuid::now_v7());
    let mut graph = comp_with(10, Vec::new());
    let wired = CompGraph {
        notes: Vec::new(),
        ties: Vec::new(),
        looks: Default::default(),
        nodes: vec![
            GraphNode::Read {
                id: read,
                item,
                custom_name: None,
            },
            GraphNode::Output { id: out },
        ],
        edges: vec![GraphEdge {
            from: read,
            from_port: "output".into(),
            to: out,
            to_port: "input".into(),
        }],
        layout: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
    };
    graph.graph = Some(wired.clone());
    let graph_id = graph.id;
    doc.items.push(ProjectItem::Composition(graph));
    let (g1, g2) = (placed(graph_id, 0), placed(graph_id, 2));
    let ids = [g1.id, g2.id];
    let (_, draws) = built(&doc, &comp_with(10, vec![g1, g2]), 1.0, &probes);
    assert_eq!(ids.map(|l| shows(of(&draws, l))), [10, 30]);

    let mut inst = lumit_core::fx::builtins::instantiate("node_graph").unwrap();
    lumit_core::fx::effects::node_graph::bind(&mut inst, graph_id, &wired);
    let applying = |ahead: i64| {
        let mut l = text_layer(0.0);
        starting(&mut l, -ahead);
        l.effects = vec![inst.clone()];
        l
    };
    let (h1, h2) = (applying(0), applying(2));
    let hosts = [h1.clone(), h2.clone()];
    let (jobs, draws) = built(&doc, &comp_with(10, hosts.to_vec()), 1.0, &probes);
    assert_eq!(jobs, 2, "one decode for each host's time");
    for (host, frame) in [(h1, 10), (h2, 30)] {
        let draw = draws.iter().find(|d| d.layer == host.id);
        assert_eq!(draw.map(|d| read_of(&d.graph_fx[0])), Some(frame));
    }
}
