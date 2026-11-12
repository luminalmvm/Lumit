use super::*;
use crate::meter::{MeterAcc, Meters, SLOTS};
use crate::mix::GainEnvelope;
use crate::plan::{MixNode, MixSend};
use crate::stream::Source;
use std::sync::Arc;

fn whole(rate: u32, samples: Vec<f32>) -> Arc<Source> {
    Source::whole(Arc::new(lumit_media::audio::AudioBuffer { rate, samples }))
}

fn clip(source: &Arc<Source>, start_frame: i64, src_start: usize, len: usize) -> PlacedClip {
    PlacedClip {
        source: Arc::clone(source),
        start_frame,
        src_start,
        len,
        gain: [1.0, 1.0],
        envelope: None,
        node: 0,
        silent: false,
    }
}

/// **The callback's mix is the frame-at-a-time mix**, sample for sample, with
/// the racks dry and the limiter off: the plan is asked once per block where
/// [`MixPlan::frame_at`] asks it once per frame, and that must not change a
/// single sample of what is heard.
///
/// The plan has everything a cut has. A clip that starts before the comp
/// does, a gap, two clips crossfading under their own envelopes, a hot
/// overlap the ceiling has to hold, a clip straight to the master, a strip
/// into a bus and a keyed fader with a send, and a file read in blocks, twice
/// over and across a block's seam. It is read whole, then in stretches of odd
/// lengths from an odd place, which is what a seek leaves the callback doing.
#[test]
fn the_buffer_mix_is_the_frame_mix_over_gaps_overlaps_and_a_seek() {
    use crate::stream::tests::{ramp_wav, Scratch};
    const RATE: u32 = 8_000;
    let dir = Scratch::new("mix");
    let path = ramp_wav(&dir.0, "ramp.wav", RATE, 5);
    let pool = crate::stream::Pool::new(64 * 1024 * 1024);
    let file = pool.file(&path, RATE);
    let block = crate::stream::BLOCK_SECONDS * RATE as usize;

    let wave = |frames: usize, step: f32| {
        whole(
            RATE,
            (0..frames)
                .flat_map(|n| {
                    let v = (n as f32 * step).sin();
                    [v * 0.8, -v * 0.6]
                })
                .collect(),
        )
    };
    let ramp = |points: Vec<[f32; 2]>| Some(Arc::new(GainEnvelope { stride: 16, points }));
    let (a, b) = (wave(300, 0.05), wave(300, 0.11));
    let plan = MixPlan {
        clips: vec![
            // Starts before the comp does.
            clip(&a, -20, 0, 100),
            // A gap, then two clips crossing under opposed ramps.
            PlacedClip {
                envelope: ramp(vec![[1.0, 0.9], [0.7, 0.6], [0.2, 0.1], [0.0, 0.0]]),
                ..clip(&a, 200, 30, 48)
            },
            PlacedClip {
                envelope: ramp(vec![[0.0, 0.0], [0.3, 0.4], [0.8, 0.9], [1.0, 1.0]]),
                node: 1,
                ..clip(&b, 216, 10, 48)
            },
            // Two loud clips at once: over the ceiling.
            PlacedClip {
                gain: [1.5, 1.5],
                ..clip(&a, 300, 100, 60)
            },
            PlacedClip {
                gain: [1.5, 1.5],
                node: NONE,
                ..clip(&b, 310, 100, 60)
            },
            // The file, across the seam between its first two blocks, and
            // the same file again a little later and overlapping.
            PlacedClip {
                gain: [0.9, 0.7],
                node: 1,
                ..clip(&file, 400, block - 30, 80)
            },
            clip(&file, 440, 2 * block - 10, 90),
        ],
        nodes: vec![
            MixNode {
                fader: Fader::constant([0.8, 0.9]),
                out: Some(2),
                ..MixNode::default()
            },
            MixNode {
                fader: Fader {
                    envelope: ramp((0..40).map(|n| [n as f32 * 0.03, 1.0]).collect()),
                    ..Fader::default()
                },
                sends: vec![MixSend {
                    to: 2,
                    level: Fader::constant([0.5, 0.5]),
                    pre: false,
                    on: true,
                }],
                ..MixNode::default()
            },
            MixNode {
                fader: Fader::constant([1.1, 1.1]),
                ..MixNode::default()
            },
        ],
        total_frames: 560,
        master: Fader::constant([0.9, 0.9]),
        ..MixPlan::default()
    }
    .finish()
    .unwrap();
    let frames_of = |plan: &MixPlan| -> Vec<(f32, f32)> {
        (0..plan.total_frames).map(|i| plan.frame_at(i)).collect()
    };
    let mut work = MixWork::new(RATE);
    work.set_limiter(false);
    let mut mix = |at: usize, out: &mut [f32], acc: Option<&mut MeterAccs>| {
        plan.mix_block(at, out, &mut work, &mut DrySource, acc, None)
    };

    // Before anything of the file is decoded: its clips are silence, and
    // the mix says how much of them was due.
    let mut out = vec![1.0f32; plan.total_frames * 2];
    assert_eq!(
        mix(0, &mut out, None),
        80 + 90,
        "every frame of both file clips was due"
    );
    let cold = frames_of(&plan);
    assert!(out.chunks_exact(2).map(|f| (f[0], f[1])).eq(cold.clone()));
    assert_eq!(cold[450], (0.0, 0.0));

    // Filled, as the thread that fills ahead leaves it.
    plan.warm(0, plan.total_frames);
    let want = frames_of(&plan);
    assert_ne!(want[450], (0.0, 0.0), "the file sounds once it is there");
    assert_eq!(mix(0, &mut out, None), 0);
    assert!(out.chunks_exact(2).map(|f| (f[0], f[1])).eq(want.clone()));
    assert!(
        want.iter().any(|f| f.0 == crate::mix::MASTER_CEILING),
        "the hot overlap reached the ceiling"
    );

    // From an odd place in stretches of odd lengths.
    let mut at = 137;
    for len in [1usize, 7, 64, 3, 250, 99] {
        let len = len.min(plan.total_frames - at);
        let mut out = vec![0.0f32; len * 2];
        let mut acc = [MeterAcc::default(); SLOTS];
        mix(at, &mut out, Some(&mut acc));
        assert!(
            out.chunks_exact(2)
                .map(|f| (f[0], f[1]))
                .eq(want[at..at + len].iter().copied()),
            "{len} frames from {at}"
        );
        at += len;
    }

    // And the blocks are the file: the same plan over the file decoded
    // whole is the same sound.
    let decoded = whole(
        RATE,
        lumit_media::audio::decode_all(&path, RATE).unwrap().samples,
    );
    let mut over_whole = plan.clone();
    for c in over_whole.clips.iter_mut().filter(|c| c.source.is_file()) {
        c.source = Arc::clone(&decoded);
    }
    assert!(frames_of(&over_whole) == want);
}

/// A layer routed to a bus, with a pre-fader and a post-fader send to a
/// second bus, reaches the master at the gains the desk says. Each node's
/// meter reads its own sum after its fader, a bus's included. Switching the
/// layer's output off silences that edge only: the sends still play and the
/// first bus falls silent.
#[test]
fn a_layer_reaches_the_master_through_its_bus_and_sends() {
    let layer = whole(48_000, vec![0.4; 64 * 2]);
    let mut plan = MixPlan {
        clips: vec![clip(&layer, 0, 0, 64)],
        nodes: vec![
            MixNode {
                fader: Fader::constant([0.5, 0.5]),
                out: Some(1),
                sends: vec![
                    MixSend {
                        to: 2,
                        level: Fader::constant([0.25, 0.25]),
                        pre: true,
                        on: true,
                    },
                    MixSend {
                        to: 2,
                        level: Fader::constant([0.1, 0.1]),
                        pre: false,
                        on: true,
                    },
                    MixSend {
                        to: 2,
                        level: Fader::constant([1.0, 1.0]),
                        pre: false,
                        on: false,
                    },
                ],
                ..MixNode::default()
            },
            MixNode {
                fader: Fader::constant([0.8, 0.8]),
                ..MixNode::default()
            },
            MixNode::default(),
        ],
        total_frames: 64,
        master: Fader::constant([0.5, 0.5]),
        ..MixPlan::default()
    };
    let run = |plan: MixPlan| {
        let plan = plan.finish().unwrap();
        let mut work = MixWork::new(48_000);
        work.set_limiter(false);
        let mut acc = [MeterAcc::default(); SLOTS];
        let mut out = vec![0.0f32; 64 * 2];
        plan.mix_block(0, &mut out, &mut work, &mut DrySource, Some(&mut acc), None);
        let meters = Meters::default();
        meters.publish(&acc, plan.nodes.len());
        (out, meters)
    };
    let near = |got: f32, want: f32| (got - want).abs() < 1e-6;

    // Through the bus 0.4 × 0.5 × 0.8; pre send 0.4 × 0.25; post send
    // 0.4 × 0.5 × 0.1; all through the master's half.
    let (out, meters) = run(plan.clone());
    assert!(
        out.iter().all(|v| near(*v, (0.16 + 0.1 + 0.02) * 0.5)),
        "{}",
        out[0]
    );
    assert!(
        near(meters.read(0).peak[0], 0.2),
        "the layer after its fader"
    );
    assert!(
        near(meters.read(1).peak[0], 0.16),
        "the bus meters its own sum"
    );
    assert!(near(meters.read(2).peak[0], 0.12), "and so does the other");
    assert!(near(meters.read(meter::MASTER).peak[0], 0.14));

    plan.nodes[0].out_on = false;
    let (out, meters) = run(plan);
    assert!(out.iter().all(|v| near(*v, 0.12 * 0.5)), "{}", out[0]);
    assert!(near(meters.read(0).peak[0], 0.2), "the layer still meters");
    assert_eq!(meters.read(1).peak[0], 0.0, "the bus it fed is silent");
    assert!(near(meters.read(2).peak[0], 0.12), "the sends still play");
}
