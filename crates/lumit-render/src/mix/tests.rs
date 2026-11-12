use lumit_audio::stream::Source;
use lumit_core::model::Document;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use super::bake::{job_bake, volume_bake};
use super::jobs::{AudioChain, AudioJob, Carrier, ClipFade, ExpressionScope};
use super::mixdown::{audio_samples_through, mixdown_prepared};
use crate::headless::tests::{push_comp, push_footage_item, push_layer, seed_has_audio};
use crate::headless::AudioJobsBuilder;
use lumit_core::model::{LayerKind, ProjectItem};

/// Volume baking (docs/09 §6): a static Volume is exactly its constant
/// gain; a keyframed fade becomes a control-rate envelope sampled in
/// layer time (comp time − start offset), falling to true zero at the
/// −inf knee.
#[test]
fn volume_bake_static_gain_and_animated_envelope() {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::time::Rational;
    let job = |volume: Property, offset_s: f64| AudioJob {
        item: uuid::Uuid::nil(),
        layer: uuid::Uuid::nil(),
        clip: None,
        sound: lumit_media::audio::SoundSource::file(PathBuf::new()),
        in_s: 0.0,
        out_s: 10.0,
        offset_s,
        volume,
        pan: Property::zero(),
        carriers: Vec::new(),
        fade: None,
        driven: None,
        chain: None,
        clip_chain: None,
        retime: None,
        scope: None,
    };
    let (g, env) = volume_bake(&job(Property::fixed(-6.0), 0.0), 0, 48_000, 48_000);
    assert!(env.is_none(), "static volume needs no envelope");
    assert!((g[0] - 0.501_19).abs() < 1e-3);
    assert!((g[1] - g[0]).abs() < 1e-9, "centred: both channels alike");

    // A 1 s fade 0 dB → −inf, on a layer whose time 0 sits at comp 1 s.
    let key = |t: i64, v: f64| Keyframe {
        time: Rational::new(t, 1).unwrap(),
        value: v,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let fade = Property {
        animation: Animation::Keyframed(vec![key(0, 0.0), key(1, -100.0)]),
        extra: serde_json::Map::new(),
    };
    // Placed at comp 1 s (start_frame 48000), offset 1 s: layer time 0..1.
    let (g, env) = volume_bake(&job(fade.clone(), 1.0), 48_000, 48_000, 48_000);
    assert_eq!(g, [1.0, 1.0]);
    let env = env.unwrap();
    assert!(
        (env.gain_at(0)[0] - 1.0).abs() < 1e-6,
        "fade starts at unity"
    );
    assert!(
        env.gain_at(0)[0] > env.gain_at(24_000)[0]
            && env.gain_at(24_000)[0] > env.gain_at(47_500)[0],
        "the fade descends"
    );
    assert_eq!(
        env.gain_at(48_000),
        [0.0, 0.0],
        "the −inf knee lands at silence"
    );

    // Carrier chain (precomp audio): the precomp layer's −6 dB multiplies
    // the inner layer's −6 dB — two static links stay a constant product.
    let mut carried = job(Property::fixed(-6.0), 0.0);
    carried.carriers = vec![Carrier {
        volume: Property::fixed(-6.0),
        pan: Property::zero(),
        offset_s: 0.0,
        fade: None,
        chain: None,
        retime: None,
        scope: None,
    }];
    let (g, env) = volume_bake(&carried, 0, 48_000, 48_000);
    assert!(env.is_none());
    assert!(
        (g[0] - 0.251_19).abs() < 1e-3,
        "gains multiply through the chain"
    );
    // An animated carrier envelopes the whole chain.
    let mut fading_carrier = job(Property::fixed(0.0), 0.0);
    fading_carrier.carriers = vec![Carrier {
        volume: fade,
        pan: Property::zero(),
        offset_s: 0.0,
        fade: None,
        chain: None,
        retime: None,
        scope: None,
    }];
    let (_, env) = volume_bake(&fading_carrier, 0, 48_000, 48_000);
    assert!(env.is_some(), "an animated carrier forces the envelope");

    // **Pan is the same stage**: a hard-right balance silences the
    // left channel and lifts the right by the constant-power √2, and a
    // Precomp layer's own balance multiplies channel by channel.
    let mut right = job(Property::fixed(0.0), 0.0);
    right.pan = Property::fixed(lumit_audio::mix::PAN_FULL);
    let (g, env) = volume_bake(&right, 0, 48_000, 48_000);
    assert!(env.is_none(), "a static pan needs no envelope either");
    assert!(g[0].abs() < 1e-6, "nothing left of a hard-right layer");
    assert!((g[1] - std::f32::consts::SQRT_2).abs() < 1e-5);

    let mut opposed = job(Property::fixed(0.0), 0.0);
    opposed.pan = Property::fixed(-lumit_audio::mix::PAN_FULL);
    opposed.carriers = vec![Carrier {
        volume: Property::zero(),
        pan: Property::fixed(lumit_audio::mix::PAN_FULL),
        offset_s: 0.0,
        fade: None,
        chain: None,
        retime: None,
        scope: None,
    }];
    let (g, _) = volume_bake(&opposed, 0, 48_000, 48_000);
    assert!(
        g[0].abs() < 1e-6 && g[1].abs() < 1e-6,
        "hard left inside a hard-right precomp leaves nothing on either side"
    );

    // An animated pan forces the envelope even with a static Volume, and
    // the two channels then move independently.
    let mut sweeping = job(Property::fixed(0.0), 0.0);
    sweeping.pan = Property {
        animation: Animation::Keyframed(vec![key(0, -100.0), key(1, 100.0)]),
        extra: serde_json::Map::new(),
    };
    let (_, env) = volume_bake(&sweeping, 0, 48_000, 48_000);
    let env = env.expect("an animated pan forces the envelope");
    assert!(env.gain_at(0)[0] > env.gain_at(0)[1], "starts on the left");
    assert!(
        env.gain_at(47_000)[1] > env.gain_at(47_000)[0],
        "and ends on the right"
    );

    // An expression on the Volume is read at every control point, in its
    // layer's comp time: here 0 dB at the comp's start falling to the knee a
    // second later, on a comp that starts 1 s into the mix.
    let mut driven = job(Property::zero(), 1.0);
    driven
        .volume
        .set_animation(Animation::Expression("-100.0 * time".into()));
    driven.scope = Some(Arc::new(ExpressionScope {
        doc: Arc::new(Document::new()),
        comp: Uuid::nil(),
        layer: Uuid::nil(),
        base_s: 1.0,
    }));
    let (_, env) = volume_bake(&driven, 48_000, 48_000, 48_000);
    let env = env.expect("an expression on the Volume forces the envelope");
    assert!(
        (env.gain_at(0)[0] - 1.0).abs() < 1e-3,
        "unity at comp time 0"
    );
    assert_eq!(env.gain_at(48_000), [0.0, 0.0], "silent a second later");
}

/// A **clip crossfade** is two opposed equal-power ramps: each
/// clip's own envelope, so sliding one moves its ramp with it, and the
/// pair holds level across the join rather than dipping in the middle the
/// way a straight line would.
#[test]
fn a_clip_join_is_two_opposed_equal_power_ramps() {
    // Clip A runs 0..2 and fades out over its last second; clip B runs
    // 1..3 and fades in over its first.
    let out = ClipFade {
        start_s: 0.0,
        head_s: 0.0,
        end_s: 2.0,
        tail_s: 1.0,
        ..Default::default()
    };
    let into = ClipFade {
        start_s: 1.0,
        head_s: 1.0,
        end_s: 3.0,
        tail_s: 0.0,
        ..Default::default()
    };
    assert!(out.is_active() && into.is_active());
    assert!((out.gain_at(0.5) - 1.0).abs() < 1e-6, "before the join");
    assert!((into.gain_at(2.5) - 1.0).abs() < 1e-6, "after it");
    assert!(out.gain_at(2.0).abs() < 1e-6, "A is gone by its end");
    assert!(into.gain_at(1.0).abs() < 1e-6, "B starts from nothing");

    // Across the overlap the two powers sum to one at every point, which
    // is what "equal power" buys: no dip in the middle of the dissolve.
    for n in 0..=10 {
        let t = 1.0 + f64::from(n) / 10.0;
        let (a, b) = (out.gain_at(t), into.gain_at(t));
        assert!(
            (a * a + b * b - 1.0).abs() < 1e-5,
            "t={t}: {a}² + {b}² is not one"
        );
    }
    // A clip with no neighbour has no ramp and costs nothing.
    assert!(!ClipFade::default().is_active());
    assert_eq!(ClipFade::default().gain_at(5.0), 1.0);
}

/// **Preview and export hear the same mix**, with a pan sweep and
/// a clip crossfade ramp on it — the two later additions to the gain
/// stage.
///
/// One job, one bake, and then the two mixers that exist: the exporter's
/// `mix_stereo_at` over a `PlacedAudio`, and playback's `MixPlan` over the
/// `PlacedClip` the bridge builds from the same bake. They are different
/// loops — one writes a whole buffer, one answers a frame at a time from
/// the realtime callback — so "they agree" is a claim that has to be
/// checked rather than assumed.
#[test]
fn a_panned_and_faded_clip_mixes_the_same_in_both_paths() {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::time::Rational;
    use std::sync::Arc;

    let rate = 48_000u32;
    let frames = rate as usize; // one second
                                // A source that is not symmetrical, so a pan that swapped the channels
                                // would be caught rather than looking identical.
    let samples: Vec<f32> = (0..frames)
        .flat_map(|n| {
            let t = n as f32 / frames as f32;
            [0.6 * (t * 9.0).sin(), 0.4 * (t * 5.0).cos()]
        })
        .collect();

    let key = |t: i64, v: f64| Keyframe {
        time: Rational::new(t, 1).expect("time"),
        value: v,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let job = AudioJob {
        item: uuid::Uuid::nil(),
        layer: uuid::Uuid::nil(),
        clip: None,
        sound: lumit_media::audio::SoundSource::file(PathBuf::new()),
        in_s: 0.0,
        out_s: 1.0,
        offset_s: 0.0,
        volume: Property::fixed(-3.0),
        // Sweeping hard left to hard right across the second.
        pan: Property {
            animation: Animation::Keyframed(vec![key(0, -100.0), key(1, 100.0)]),
            extra: serde_json::Map::new(),
        },
        carriers: Vec::new(),
        // And rising out of a join over its first quarter second.
        fade: Some(ClipFade {
            start_s: 0.0,
            head_s: 0.25,
            end_s: 1.0,
            tail_s: 0.0,
            ..Default::default()
        }),
        driven: None,
        chain: None,
        clip_chain: None,
        retime: None,
        scope: None,
    };

    let (gain, envelope) = volume_bake(&job, 0, frames, rate);
    let envelope = envelope.expect("a sweep and a ramp both force the envelope");
    let baked = lumit_audio::mix::mix_stereo_at(
        &[lumit_audio::mix::PlacedAudio {
            start_frame: 0,
            samples: &samples,
            gain,
            envelope: Some(envelope.clone()),
        }],
        frames,
        1.0,
    );
    let plan = lumit_audio::plan::MixPlan::single(
        vec![lumit_audio::plan::PlacedClip {
            source: Source::whole(Arc::new(lumit_media::audio::AudioBuffer {
                rate,
                samples: samples.clone(),
            })),
            start_frame: 0,
            src_start: 0,
            len: frames,
            gain,
            envelope: Some(Arc::new(envelope)),
            node: 0,
            silent: false,
        }],
        frames,
    );
    for i in (0..frames).step_by(97) {
        let (l, r) = plan.frame_at(i);
        assert!(
            (l - baked[i * 2]).abs() < 1e-6 && (r - baked[i * 2 + 1]).abs() < 1e-6,
            "frame {i}: playback ({l}, {r}) vs export ({}, {})",
            baked[i * 2],
            baked[i * 2 + 1]
        );
    }

    // And the mix really is doing both things, so the agreement above is
    // not two paths agreeing on having ignored them.
    assert!(
        baked[0].abs() < 1e-4 && baked[1].abs() < 1e-4,
        "the head ramp starts from silence"
    );
    let quarter = frames / 4;
    let (l0, r0) = plan.frame_at(quarter);
    let (l1, r1) = plan.frame_at(frames - 2);
    assert!(
        l0.abs() > r0.abs() && r1.abs() > l1.abs(),
        "the sweep starts on the left and ends on the right"
    );
}

/// The A/V interleave rule: cumulative rounding never drifts, and the
/// total after all frames equals the whole soundtrack.
#[test]
fn audio_samples_through_never_drifts() {
    let (fps, rate) = (60.0, 48_000u32);
    // 60 fps at 48 kHz is exactly 800 samples per frame.
    assert_eq!(audio_samples_through(1, fps, rate), 800);
    assert_eq!(audio_samples_through(300, fps, rate), 240_000);
    // An awkward rate: 29.97 fps. Per-frame chunks vary by ±1 sample but
    // the cumulative total stays glued to the exact value.
    let fps = 30_000.0 / 1001.0;
    let mut prev = 0;
    for n in 1..=1000 {
        let now = audio_samples_through(n, fps, rate);
        let chunk = now - prev;
        assert!((1601..=1602).contains(&chunk), "frame {n} chunk {chunk}");
        let exact = n as f64 / fps * 48_000.0;
        assert!((now as f64 - exact).abs() <= 0.5, "frame {n} drifted");
        prev = now;
    }
    // Degenerate input answers zero, never panics.
    assert_eq!(audio_samples_through(100, 0.0, rate), 0);
}

/// One built-in audio effect on a rack of its own, with `over` written
/// into its rows.
///
/// A chain with no wires in it, so the document and the two ids are never
/// read: `bake_values` only builds an expression context for a parameter a
/// wire feeds.
fn audio_rack(match_name: &str, over: &[(&str, f64)]) -> Arc<AudioChain> {
    rack_of(vec![audio_effect(match_name, over)])
}

/// One built-in audio effect, with `over` written into its rows.
fn audio_effect(match_name: &str, over: &[(&str, f64)]) -> lumit_core::model::EffectInstance {
    use lumit_core::anim::Property;
    use lumit_core::model::EffectValue;

    let mut instance =
        lumit_core::fx::builtins::instantiate(match_name).expect("a catalogue entry");
    for param in &mut instance.params {
        if let Some((_, value)) = over.iter().find(|(id, _)| *id == param.id) {
            param.value = EffectValue::Float(Property::fixed(*value));
        }
    }
    instance
}

/// A rack of them.
fn rack_of(effects: Vec<lumit_core::model::EffectInstance>) -> Arc<AudioChain> {
    Arc::new(AudioChain {
        doc: Arc::new(Document::new()),
        comp: Uuid::nil(),
        layer: Uuid::nil(),
        effects,
        graph: lumit_core::graph::LayerGraph::default(),
        offset_s: 0.0,
        base_s: 0.0,
    })
}

/// A job carrying the two racks and nothing else. Both are `None` for a
/// layer whose switches are off, which is what
/// `a_clip_has_its_own_chain_and_each_fx_switch_drops_one` in
/// `crate::headless` pins the switches to.
fn racked_job(clip: Option<Arc<AudioChain>>, row: Option<Arc<AudioChain>>) -> AudioJob {
    AudioJob {
        item: Uuid::nil(),
        layer: Uuid::nil(),
        clip: None,
        sound: lumit_media::audio::SoundSource::file(PathBuf::new()),
        in_s: 0.0,
        out_s: 1.0,
        offset_s: 0.0,
        volume: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
        carriers: Vec::new(),
        fade: None,
        driven: None,
        chain: row,
        clip_chain: clip,
        retime: None,
        scope: None,
    }
}

/// The loudest sample in a run, either channel.
fn loudest(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |top, s| top.max(s.abs()))
}

/// **The clip's rack runs ahead of the row's, and a rack that is gone is
/// simply not run** (docs/impl/audio-effects.md §6 plan 4,
/// docs/impl/audio-timeline.md §4).
///
/// A gain of +6 dB on the clip and a limiter on the row. The gain drives
/// the sound over the ceiling and the limiter, hearing it afterwards on the
/// layer's node, holds it there; the other order would let the +6 straight
/// out. So the peak alone says which way round the two racks ran.
///
/// Each rack arrives as `None` when its switch is off: the clip's `fx`
/// and the layer's `switches.fx`, both pinned where the jobs are built.
/// This is the other half of that, because a chain that is gone changes
/// the sound back.
#[test]
fn a_clips_rack_runs_ahead_of_the_rows_and_a_missing_rack_is_not_run() {
    let rate = 48_000u32;
    let frames = rate as usize / 4;
    // A quarter second at half scale, which is a whisker under the −6 dB
    // ceiling the limiter is set to below.
    let input = tone(rate, frames, 0.5);
    let ceiling = 10f32.powf(-6.0 / 20.0);
    let gain = || Some(audio_rack("audio_gain", &[("gain", 6.0)]));
    let limiter = || vec![audio_effect("audio_limiter", &[("ceiling", -6.0)])];

    let mut doc = Document::new();
    let song = push_footage_item(&mut doc, "song.wav");
    seed_has_audio(song);
    let comp = push_comp(&mut doc, "mix", 32, 32);
    push_layer(&mut doc, comp, LayerKind::Footage { item: song });
    let through = |clip: Option<Arc<AudioChain>>, row: Vec<lumit_core::model::EffectInstance>| {
        let mut doc = doc.clone();
        if let Some(ProjectItem::Composition(c)) = doc.item_mut(comp) {
            c.layers[0].effects = row;
        }
        let doc = Arc::new(doc);
        let c = doc.comp(comp).unwrap().clone();
        let mut job = AudioJobsBuilder::new().audio_jobs(&doc, &c).remove(0);
        job.out_s = 0.25;
        job.clip_chain = clip;
        mixdown_prepared(&doc, &c, &[(Arc::clone(&input), job)], rate, 0.25)
    };

    let both = through(gain(), limiter());
    assert!(
        loudest(&both) < ceiling * 1.05,
        "the limiter heard the +6 dB: peak {}",
        loudest(&both)
    );

    // The clip's rack alone: the +6 dB comes out untouched, which is what
    // the other order would have produced above.
    let clip_only = through(gain(), Vec::new());
    assert!(
        loudest(&clip_only) > ceiling * 1.8,
        "nothing held the gain back: peak {}",
        loudest(&clip_only)
    );

    // The row's rack alone: the sound was already under the ceiling, so
    // the limiter leaves it where it was.
    let row_only = through(None, limiter());
    assert!(loudest(&row_only) < ceiling * 1.05);
    assert!(loudest(&row_only) > ceiling * 0.8, "and it was heard");

    // And neither rack is the mix untouched, byte for byte.
    assert!(
        job_bake(
            &racked_job(None, None),
            &input.samples,
            0,
            rate,
            true,
            &std::sync::atomic::AtomicBool::new(false)
        )
        .is_none(),
        "no rack, no bake"
    );
}

/// A quarter-second-ish 220 Hz tone at `level`, both sides.
fn tone(rate: u32, frames: usize, level: f32) -> Arc<lumit_media::audio::AudioBuffer> {
    Arc::new(lumit_media::audio::AudioBuffer {
        rate,
        samples: (0..frames)
            .flat_map(|n| {
                let phase = std::f64::consts::TAU * 220.0 * n as f64 / f64::from(rate);
                [phase.sin() as f32 * level; 2]
            })
            .collect(),
    })
}

/// **A Precomp layer's rack hears the sum, and the layer's Volume rides
/// on what comes back** (docs/09 §3.1): the rack runs on the layer's node.
///
/// Two copies of the same half-scale tone play inside a comp placed through
/// one Precomp layer whose rack is a limiter set to −6 dB. Either one alone
/// is already under that ceiling, so only the sum can move the limiter: if
/// the rack were run per source the two would come out untouched and add to
/// full scale. The peak alone therefore says whether the sum existed.
///
/// Then the same mix with the Precomp layer at −6 dB. The fader is
/// **after** the insert, so it halves the held sum; riding inside the rack
/// it would have taken each source under the ceiling and the limiter would
/// never have bitten.
#[test]
fn a_precomp_layers_rack_hears_the_summed_comp() {
    let rate = 48_000u32;
    let frames = rate as usize / 4;
    let ceiling = 10f32.powf(-6.0 / 20.0);
    let half = tone(rate, frames, 0.5);

    let mut doc = Document::new();
    let song = push_footage_item(&mut doc, "song.wav");
    seed_has_audio(song);
    let inner = push_comp(&mut doc, "inner", 32, 32);
    push_layer(&mut doc, inner, LayerKind::Footage { item: song });
    push_layer(&mut doc, inner, LayerKind::Footage { item: song });
    let outer = push_comp(&mut doc, "outer", 32, 32);
    push_layer(&mut doc, outer, LayerKind::Precomp { comp: inner });
    let through = |volume_db: f64, racked: bool| {
        let mut doc = doc.clone();
        if let Some(ProjectItem::Composition(c)) = doc.item_mut(outer) {
            c.layers[0].volume_db = lumit_core::anim::Property::fixed(volume_db);
            if racked {
                c.layers[0].effects = vec![audio_effect("audio_limiter", &[("ceiling", -6.0)])];
            }
        }
        let doc = Arc::new(doc);
        let c = doc.comp(outer).unwrap().clone();
        let decoded: Vec<_> = AudioJobsBuilder::new()
            .audio_jobs(&doc, &c)
            .into_iter()
            .map(|job| (Arc::clone(&half), job))
            .collect();
        assert_eq!(decoded.len(), 2);
        mixdown_prepared(&doc, &c, &decoded, rate, 0.25)
    };

    let held = through(0.0, true);
    assert!(
        loudest(&held) < ceiling * 1.05 && loudest(&held) > ceiling * 0.5,
        "the limiter heard both sources at once: peak {}",
        loudest(&held)
    );

    let quieter = through(-6.0, true);
    assert!(
        (loudest(&quieter) - loudest(&held) * ceiling).abs() < 0.02,
        "the Precomp layer's Volume rides on what the rack gave back: \
         peak {} against {}",
        loudest(&quieter),
        loudest(&held)
    );

    // And with no rack nothing is summed early: the two sources reach full
    // scale and the master's own limiter holds them.
    let open = through(0.0, false);
    assert!(
        loudest(&open) > ceiling * 1.5,
        "a Precomp layer with no rack places its sources as it always did: \
         peak {}",
        loudest(&open)
    );
}

/// **An export made to a loudness lands on it**: the mix is measured, the
/// master moved by the difference and the limiter holds the true peak under
/// the ceiling, so a delivery spec is met without a second tool.
///
/// A quiet tone with a short loud burst five times a second: lifting it to
/// the target pushes every burst well past the ceiling, and the limiter
/// takes back more than a unit of the lift, which a second measure puts
/// right.
#[test]
fn an_export_made_to_a_loudness_lands_within_half_a_unit() {
    let rate = 48_000u32;
    let frames = 3 * rate as usize;
    let mut samples = tone(rate, frames, 0.07).samples.clone();
    for start in (0..frames).step_by(rate as usize / 5) {
        for n in start..(start + rate as usize / 500).min(frames) {
            let phase = std::f32::consts::TAU * 1000.0 * (n - start) as f32 / rate as f32;
            samples[n * 2] += phase.sin() * 0.6;
            samples[n * 2 + 1] += phase.sin() * 0.6;
        }
    }
    let clicks = Arc::new(lumit_media::audio::AudioBuffer { rate, samples });
    let mut doc = Document::new();
    let song = push_footage_item(&mut doc, "song.wav");
    seed_has_audio(song);
    let comp = push_comp(&mut doc, "mix", 32, 32);
    push_layer(&mut doc, comp, LayerKind::Footage { item: song });
    let doc = Arc::new(doc);
    let c = doc.comp(comp).unwrap().clone();
    let jobs = AudioJobsBuilder::new().audio_jobs(&doc, &c);
    let source = Source::whole(Arc::clone(&clicks));
    let (mut plan, _) = super::graph::build_graph(
        &doc,
        &c,
        jobs,
        &|_| Some(Arc::clone(&source)),
        rate,
        3.0,
        &mut super::live::RackBakes::default(),
        None,
        true,
        &std::sync::atomic::AtomicBool::new(false),
        None,
    )
    .unwrap();

    let (lufs, true_peak_db) = (-16.0, -1.0);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let measured = super::render::measure(&plan, rate, &stop, &mut |_| {})
        .unwrap()
        .integrated;
    assert!(measured < lufs - 3.0, "the tone starts quieter: {measured}");
    let lifted = 0.6 * 10f64.powf((lufs - measured) / 20.0);
    assert!(lifted > 1.0, "the bursts go past the ceiling: {lifted}");
    let mut once = plan.clone();
    once.ceiling = lumit_audio::mix::db_to_gain(true_peak_db);
    super::render::lift(&mut once, lufs - measured);
    let once = super::render::measure(&once, rate, &stop, &mut |_| {})
        .unwrap()
        .integrated;
    assert!(
        lufs - once > 1.0,
        "one lift by the difference lands over a unit short: {once}"
    );
    super::render::aim(&mut plan, rate, lufs, true_peak_db, &stop, &mut |_| {}).unwrap();
    let out = super::render::render(Arc::new(plan), rate, true, &mut |_| {});

    let mut meter = lumit_audio::loudness::Loudness::new(rate);
    meter.add(&out);
    let reading = meter.reading();
    assert!(
        (f64::from(reading.integrated) - lufs).abs() < 0.5,
        "integrated {} for a target of {lufs}",
        reading.integrated
    );
    let peak_db = 20.0 * f64::from(reading.true_peak).log10();
    assert!(
        peak_db <= true_peak_db + 0.05,
        "true peak {peak_db} dB over the {true_peak_db} dB ceiling"
    );
}

/// **A click through the distortion lands at the same comp time as
/// without it** (plan 2).
///
/// The oversampler's half-band pair is linear phase, so the whole click
/// arrives late by one fixed number of frames. The bake reports it and the
/// mixer places the run that many frames earlier, which puts the click
/// back where it was.
#[test]
fn a_click_through_the_distortion_lands_at_the_same_comp_time() {
    let rate = 48_000u32;
    let frames = 4_096usize;
    let at = 1_000usize;
    let mut input = vec![0.0f32; frames * 2];
    input[at * 2] = 0.2;
    input[at * 2 + 1] = 0.2;

    let job = racked_job(
        None,
        Some(audio_rack("audio_distortion", &[("drive", 0.0)])),
    );
    let (wet, latency) = job_bake(
        &job,
        &input,
        0,
        rate,
        true,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("the distortion opens");
    assert!(latency > 0, "the half-band pair's delay is reported");
    let peak = (0..wet.len() / 2)
        .max_by(|a, b| wet[a * 2].abs().total_cmp(&wet[b * 2].abs()))
        .expect("a frame to be loudest");
    assert_eq!(
        peak as i64 - i64::from(latency),
        at as i64,
        "placed {latency} frames earlier the click is back where it started"
    );
}
