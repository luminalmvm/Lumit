//! The bridge audio tests: the jobs signature, the plan, the output
//! choice and the audio insert chain.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use lumit_core::anim::Property;

/// Tests that write the one shared audio state take this first, so two of
/// them never see each other's half-set state.
fn state_tests() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An empty comp to build in: the jobs the tests hand in name layers it
/// does not have, so each layer they name is a strip of its own.
fn bare() -> (Arc<Document>, lumit_core::model::Composition) {
    use lumit_core::time::{Duration, FrameRate, Rational};
    let comp = lumit_core::model::Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Mix".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).expect("30 fps"),
        duration: Duration(Rational::new(10, 1).expect("10 s")),
        background: lumit_core::model::LinearColour([0.0, 0.0, 0.0, 0.0]),
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    (Arc::new(Document::new()), comp)
}

/// The plan the engine would play for `jobs`, and the layer of each node.
fn plan_of(
    jobs: &[AudioJob],
    decoded: &HashMap<Uuid, Arc<lumit_media::audio::AudioBuffer>>,
    rate: u32,
    duration_s: f64,
) -> (Arc<MixPlan>, Vec<Uuid>) {
    let (doc, comp) = bare();
    let (plan, info) = build_plan(&doc, &comp, jobs, decoded, rate, duration_s);
    (plan, info.nodes.iter().map(|n| n.layer).collect())
}

/// The export's mix of `decoded`, through the desk.
fn baked(
    decoded: &[(Arc<lumit_media::audio::AudioBuffer>, AudioJob)],
    rate: u32,
    duration_s: f64,
) -> Vec<f32> {
    let (doc, comp) = bare();
    lumit_render::mix::mixdown::mixdown_prepared(&doc, &comp, decoded, rate, duration_s)
}

fn job(path: &str, in_s: f64, out_s: f64, offset_s: f64) -> AudioJob {
    let item = Uuid::now_v7();
    AudioJob {
        item,
        layer: item, // one job, one strip: the test's own layer identity
        clip: None,
        sound: lumit_media::audio::SoundSource::file(path),
        in_s,
        out_s,
        offset_s,
        volume: Property::zero(),
        pan: Property::zero(),
        carriers: Vec::new(),
        fade: None,
        driven: None,
        chain: None,
        clip_chain: None,
        retime: None,
        scope: None,
    }
}

/// The signature is the mix-change detector: identical jobs agree, and a
/// move, a trim, a Volume nudge, a removed job, or a new duration each
/// change it — the exact edits that must re-plan (docs/09 §6).
#[test]
fn the_signature_tracks_everything_that_changes_the_sound() {
    // The same rows each time: which strip a job sums into is part of the mix.
    let rows = |mut jobs: Vec<AudioJob>| {
        for (i, j) in jobs.iter_mut().enumerate() {
            j.layer = Uuid::from_u128(i as u128 + 1);
        }
        jobs
    };
    let a = rows(vec![
        job("a.mp4", 0.0, 5.0, 0.0),
        job("b.mp4", 1.0, 3.0, 1.0),
    ]);
    let same = rows(vec![
        job("a.mp4", 0.0, 5.0, 0.0),
        job("b.mp4", 1.0, 3.0, 1.0),
    ]);
    let sig = jobs_signature(&a, 10.0, 0.0);
    assert_eq!(
        sig,
        jobs_signature(&same, 10.0, 0.0),
        "identical mixes agree"
    );

    let moved = rows(vec![
        job("a.mp4", 0.5, 5.5, 0.5),
        job("b.mp4", 1.0, 3.0, 1.0),
    ]);
    assert_ne!(sig, jobs_signature(&moved, 10.0, 0.0), "a move re-plans");

    let trimmed = rows(vec![
        job("a.mp4", 0.0, 4.0, 0.0),
        job("b.mp4", 1.0, 3.0, 1.0),
    ]);
    assert_ne!(sig, jobs_signature(&trimmed, 10.0, 0.0), "a trim re-plans");

    let removed = rows(vec![job("a.mp4", 0.0, 5.0, 0.0)]);
    assert_ne!(
        sig,
        jobs_signature(&removed, 10.0, 0.0),
        "a mute/delete re-plans"
    );

    let mut louder = rows(vec![
        job("a.mp4", 0.0, 5.0, 0.0),
        job("b.mp4", 1.0, 3.0, 1.0),
    ]);
    louder[0].volume = Property::fixed(-6.0);
    assert_ne!(
        sig,
        jobs_signature(&louder, 10.0, 0.0),
        "a Volume nudge re-plans"
    );

    assert_ne!(
        sig,
        jobs_signature(&a, 12.0, 0.0),
        "a duration change re-plans"
    );
}

/// Plan building is pure: placed clips land where `place_on_timeline`
/// says, a job with no decoded source contributes nothing, and the strip
/// length follows the comp duration.
#[test]
fn build_plan_places_decoded_clips_and_skips_missing_ones() {
    let rate = 48_000u32;
    let placed = job("a.mp4", 1.0, 3.0, 1.0);
    let missing = job("gone.mp4", 0.0, 2.0, 0.0);
    let mut decoded = HashMap::new();
    decoded.insert(
        placed.item,
        Arc::new(lumit_media::audio::AudioBuffer {
            rate,
            samples: vec![0.25; 4 * rate as usize * 2], // 4 s of quiet tone
        }),
    );
    let (plan, strips) = plan_of(&[placed.clone(), missing.clone()], &decoded, rate, 5.0);
    assert_eq!(
        strips,
        vec![placed.layer, missing.layer],
        "each row with a job is a strip, the one that cannot be read meters nought"
    );
    assert_eq!(plan.clips[0].node, 0);
    assert_eq!(plan.total_frames, 5 * rate as usize);
    assert_eq!(plan.clips.len(), 1, "the undecoded job contributes nothing");
    // The layer starts at comp second 1 and sounds for its 2-second span.
    assert_eq!(plan.clips[0].start_frame, i64::from(rate));
    assert_eq!(plan.clips[0].src_start, 0);
    assert_eq!(plan.clips[0].len, 2 * rate as usize);
    // Frame 0 is silence; a frame inside the span carries the source.
    assert_eq!(plan.frame_at(0), (0.0, 0.0));
    let (l, _r) = plan.frame_at(rate as usize + 10);
    assert!((l - 0.25).abs() < 1e-6);
}

/// **The live plan runs a Precomp layer's rack over the sum** (docs/09
/// §3.1), through the same bus stage the export mixes with.
///
/// Two sources arrive through one Precomp layer whose rack is a limiter
/// set to −6 dB. Each alone is already under that ceiling, so only the sum
/// can move it. They come back as one clip on one strip, the Precomp
/// layer's own row, which is the row the mixer draws, and the sound the
/// callback would play is held at the ceiling rather than reaching full
/// scale.
#[test]
fn build_plan_sums_a_precomp_layers_rack_onto_one_strip() {
    let rate = 48_000u32;
    let ceiling = 10f32.powf(-6.0 / 20.0);
    let mut limiter = lumit_core::fx::builtins::instantiate("audio_limiter").expect("a limiter");
    for param in &mut limiter.params {
        if param.id == "ceiling" {
            param.value = lumit_core::model::EffectValue::Float(Property::fixed(-6.0));
        }
    }
    let carrier = lumit_render::mix::jobs::Carrier {
        volume: Property::zero(),
        pan: Property::zero(),
        offset_s: 0.0,
        fade: None,
        chain: Some(Arc::new(lumit_render::mix::jobs::AudioChain {
            doc: Arc::new(lumit_core::model::Document::new()),
            comp: Uuid::nil(),
            layer: Uuid::nil(),
            effects: vec![limiter],
            graph: lumit_core::graph::LayerGraph::default(),
            offset_s: 0.0,
            base_s: 0.0,
        })),
        retime: None,
        scope: None,
    };
    let row = Uuid::now_v7();
    let one = || {
        let mut j = job("a.wav", 0.0, 1.0, 0.0);
        j.layer = row;
        j.carriers = vec![carrier.clone()];
        j
    };
    let (a, b) = (one(), one());
    let mut decoded = HashMap::new();
    for j in [&a, &b] {
        decoded.insert(
            j.item,
            Arc::new(lumit_media::audio::AudioBuffer {
                rate,
                // Half scale: two of them reach full scale summed.
                samples: vec![0.5; rate as usize * 2],
            }),
        );
    }
    let (plan, strips) = plan_of(&[a, b], &decoded, rate, 1.0);
    assert_eq!(
        plan.clips.len(),
        1,
        "the two sources are summed into one bus"
    );
    assert_eq!(
        strips,
        vec![row],
        "the bus meters on the Precomp layer's row"
    );
    assert_eq!(plan.clips[0].node, 0);
    let (l, r) = plan.frame_at(rate as usize / 2);
    assert!(
        l.abs() < ceiling * 1.05 && r.abs() < ceiling * 1.05,
        "the limiter heard both sources at once: {l} and {r}"
    );
    assert!(l.abs() > 0.1, "and it is sound, not silence");
}

/// Two rows of the comp are two slots; a Precomp row's several sources
/// share one, because that is the one strip the mixer draws.
/// Past the bank's size the extras still sound and simply have no bar.
#[test]
fn meter_slots_follow_the_mixer_strips_and_the_bank_is_bounded() {
    let rate = 48_000u32;
    let tone = || {
        Arc::new(lumit_media::audio::AudioBuffer {
            rate,
            samples: vec![0.25; rate as usize * 2],
        })
    };
    // Two jobs sharing a strip (one Precomp row), then a strip of its own.
    let a = job("a.wav", 0.0, 1.0, 0.0);
    let mut b = job("b.wav", 0.0, 1.0, 0.0);
    b.layer = a.layer;
    let c = job("c.wav", 0.0, 1.0, 0.0);
    let mut decoded = HashMap::new();
    for j in [&a, &b, &c] {
        decoded.insert(j.item, tone());
    }
    let (plan, strips) = plan_of(&[a.clone(), b, c.clone()], &decoded, rate, 1.0);
    assert_eq!(strips, vec![a.layer, c.layer], "one slot per strip");
    assert_eq!(
        plan.clips.iter().map(|c| c.node).collect::<Vec<_>>(),
        vec![0, 0, 1],
        "the shared row's two sources meter onto the one slot"
    );

    // More strips than the bank holds: the first MAX_NODES are metered,
    // the rest play with no bar rather than being dropped from the mix.
    let many: Vec<AudioJob> = (0..lumit_audio::plan::MAX_NODES + 3)
        .map(|_| job("x.wav", 0.0, 1.0, 0.0))
        .collect();
    let mut decoded = HashMap::new();
    for j in &many {
        decoded.insert(j.item, tone());
    }
    let (plan, strips) = plan_of(&many, &decoded, rate, 1.0);
    assert_eq!(plan.clips.len(), many.len(), "every source still sounds");
    assert_eq!(strips.len(), lumit_audio::plan::MAX_NODES);
    assert_eq!(
        plan.clips
            .iter()
            .filter(|c| c.node == lumit_audio::plan::NONE)
            .count(),
        3,
        "the three past the bank have no bar"
    );
}

/// Choosing an output closes the open stream and forgets what was loaded,
/// so the next prepare opens the new device — and choosing the one already
/// in force does nothing at all, which is what lets the frontend hand the
/// stored choice over on every boot.
#[test]
fn choosing_an_output_closes_the_stream_and_repeating_it_does_not() {
    let _held = state_tests();
    let before = lock().device_gen;
    set_device(Some("Some device that is not here".to_owned()));
    {
        let st = lock();
        assert_eq!(st.device_gen, before.wrapping_add(1));
        assert!(matches!(st.device, Device::Untried));
        assert_eq!(st.loaded_comp, None);
        assert_eq!(st.loaded_sig, None);
        assert!(!st.playing);
    }

    set_device(Some("Some device that is not here".to_owned()));
    assert_eq!(
        lock().device_gen,
        before.wrapping_add(1),
        "the same choice again is a no-op"
    );

    // A chosen device that is not on this machine reads as a fallback —
    // unless the machine has no output at all, which says so by having no
    // active id rather than by claiming a substitution.
    let (_list, active, fell_back) = devices();
    assert_eq!(fell_back, active.is_some());

    // Put the shared state back for whatever else runs in this process.
    set_device(None);
    assert!(!devices().2, "the system default is never a fallback");
}

// ------------------------------------------- the audio insert chain --

/// A stand-in audio plugin: it multiplies by its one row and then **hard
/// clips**, which is what makes it useful for the order-of-stages test —
/// a plain gain would commute with the fader and prove nothing about where
/// the insert sits.
struct TestInsert {
    /// The block to refuse, which is what a crashed or hung plugin costs.
    refuse: Option<usize>,
    latency: u32,
}

impl lumit_core::fx::audio_chain::AudioProcessor for TestInsert {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(lumit_core::fx::params::ParamId, f64)],
        steady: i64,
    ) -> bool {
        if self.refuse == Some(steady as usize / lumit_core::fx::audio_chain::AUDIO_BLOCK_FRAMES) {
            return false;
        }
        let g = values
            .iter()
            .find(|(id, _)| *id == lumit_core::fx::params::ParamId::new("p1"))
            .map_or(1.0, |(_, v)| *v) as f32;
        for (o, i) in output.iter_mut().zip(input) {
            *o = (i * g).clamp(-0.5, 0.5);
        }
        true
    }

    fn latency(&self) -> u32 {
        self.latency
    }
}

struct TestInsertDef {
    schema: &'static lumit_core::fx::schema::EffectSchema,
    refuse: Option<usize>,
    latency: u32,
}

impl lumit_core::fx::registry::EffectDef for TestInsertDef {
    fn schema(&self) -> &'static lumit_core::fx::schema::EffectSchema {
        self.schema
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn open_audio(
        &self,
        _state: Option<Vec<u8>>,
        _values: &[(lumit_core::fx::params::ParamId, f64)],
        _rate: u32,
        _offline: bool,
    ) -> Option<Arc<dyn lumit_core::fx::audio_chain::AudioProcessor>> {
        Some(Arc::new(TestInsert {
            refuse: self.refuse,
            latency: self.latency,
        }))
    }
}

/// Register one stand-in plugin under `name`. Registration is additive and
/// by name, so each test takes a name of its own and they cannot tread on
/// one another.
fn register_insert(name: &'static str, refuse: Option<usize>, latency: u32) {
    let schema: &'static lumit_core::fx::schema::EffectSchema =
        Box::leak(Box::new(lumit_core::fx::schema::EffectSchema {
            match_name: name,
            label: "Test insert",
            version: 1,
            category: lumit_core::fx::schema::FxCategory::Utility,
            traits: lumit_core::fx::schema::EffectTraits {
                cost: lumit_core::fx::schema::CostClass::Heavy,
                roi: lumit_core::fx::schema::Roi::FullFrame,
                temporal: &[0],
                premultiplied: true,
                seeded: false,
                beat_input: false,
            },
            params: Box::leak(Box::new([lumit_core::fx::schema::ParamSchema {
                id: "p1",
                label: "Gain",
                kind: lumit_core::fx::schema::ParamKind::Slider {
                    default: 1.0,
                    range: (0.0, 4.0),
                    log: false,
                },
                unit: lumit_core::fx::params::Unit::Raw,
            }])),
            groups: &[],
            enabled_when: &[],
            matte: lumit_core::fx::schema::MatteRole::None,
        }));
    lumit_core::fx::catalogue::BUILTIN_DEFS.register(Box::leak(Box::new(TestInsertDef {
        schema,
        refuse,
        latency,
    })));
}

/// One instance of a stand-in plugin, its `p1` row holding `value`.
fn insert_instance(name: &str, value: Property) -> lumit_core::model::EffectInstance {
    lumit_core::model::EffectInstance {
        id: Uuid::now_v7(),
        effect: lumit_core::model::EffectKey {
            namespace: lumit_core::model::EffectNamespace::Clap,
            match_name: name.to_owned(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        roto: None,
        enabled: true,
        params: vec![lumit_core::model::EffectParam {
            id: "p1".into(),
            value: lumit_core::model::EffectValue::Float(value),
            extra: serde_json::Map::new(),
        }],
        sample_temporally: true,
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
        extra: serde_json::Map::new(),
    }
}

/// The chain a job carries for `effects`.
fn chain_of(
    effects: Vec<lumit_core::model::EffectInstance>,
) -> Arc<lumit_render::mix::jobs::AudioChain> {
    Arc::new(lumit_render::mix::jobs::AudioChain {
        doc: Arc::new(lumit_core::model::Document::new()),
        comp: Uuid::now_v7(),
        layer: Uuid::now_v7(),
        effects,
        graph: Default::default(),
        offset_s: 0.0,
        base_s: 0.0,
    })
}

fn tone(rate: u32, seconds: f64, value: f32) -> Arc<lumit_media::audio::AudioBuffer> {
    Arc::new(lumit_media::audio::AudioBuffer {
        rate,
        samples: vec![value; (f64::from(rate) * seconds) as usize * 2],
    })
}

/// **The insert sits ahead of Volume** (docs/impl/audio-plugins.md §2, the
/// decided position).
///
/// The fader is put down 6 dB on a layer whose plugin clips at ±0.5.
/// Insert first gives `fade × clip(x)`; fader first would give
/// `clip(fade × x)`, and with a hot source those are different numbers —
/// which is exactly why the order is decided rather than incidental.
#[test]
fn the_insert_runs_before_the_fader_not_after_it() {
    let rate = 48_000u32;
    register_insert("clap:lumit.test.prefader", None, 0);
    let source = tone(rate, 1.0, 0.8);
    let mut one = job("a.mp4", 0.0, 1.0, 0.0);
    one.volume = Property::fixed(-6.0);
    one.chain = Some(chain_of(vec![insert_instance(
        "clap:lumit.test.prefader",
        Property::fixed(1.0),
    )]));

    let baked = baked(&[(source, one)], rate, 1.0);
    let half = lumit_audio::mix::db_to_gain(-6.0);
    assert!(
        (baked[0] - 0.5 * half).abs() < 1e-5,
        "the fade rides the processed sound: expected {}, got {}",
        0.5 * half,
        baked[0]
    );
    assert!(
        (baked[0] - 0.5).abs() > 1e-3,
        "and the fader is not being swallowed by the clipper"
    );
}

/// **An animated plugin parameter renders deterministically** (the note's
/// §7 plans 4 and 7): the sweep is baked at one value per 512-frame block,
/// those blocks are a fact about the layer rather than about the playhead,
/// and two bakes of the same project are byte-identical.
#[test]
fn an_animated_plugin_parameter_bakes_per_block_and_repeats_exactly() {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    let rate = 48_000u32;
    register_insert("clap:lumit.test.sweep", None, 0);
    // 0 → 1 across the second, on a quiet source so the clipper never bites.
    let key = |t: i64, v: f64| Keyframe {
        time: lumit_core::time::Rational::new(t, 1).unwrap(),
        value: v,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let sweep = Property {
        animation: Animation::Keyframed(vec![key(0, 0.0), key(1, 1.0)]),
        extra: serde_json::Map::new(),
    };
    let source = tone(rate, 1.0, 0.2);
    let mut one = job("a.mp4", 0.0, 1.0, 0.0);
    one.chain = Some(chain_of(vec![insert_instance(
        "clap:lumit.test.sweep",
        sweep,
    )]));

    let bake = || baked(&[(Arc::clone(&source), one.clone())], rate, 1.0);
    let first = bake();
    assert_eq!(first, bake(), "two bakes of one project are identical");

    // Block b was handed the value at its own first frame, and held it for
    // the whole block — a step per 512 frames, not a slope per sample.
    let block = lumit_core::fx::audio_chain::AUDIO_BLOCK_FRAMES;
    for b in [0usize, 1, 10, 80] {
        let want = 0.2 * (b * block) as f32 / rate as f32;
        let at = b * block * 2;
        assert!(
            (first[at] - want).abs() < 1e-5,
            "block {b}: expected {want}, got {}",
            first[at]
        );
        assert!(
            (first[at] - first[at + (block - 1) * 2]).abs() < 1e-9,
            "block {b} holds one value all the way across"
        );
    }
}

/// **A block the plugin refuses ships dry, and the sound goes on**
/// (docs/impl/audio-plugins.md §3): the refused block carries the chain's
/// input unchanged, everything either side is processed, and the splice is
/// ramped rather than cut — a hole in the music would be worse than a block
/// of it slightly wrong, and a click worse than either.
#[test]
fn a_refused_block_ships_dry_and_the_rest_still_plays() {
    let rate = 48_000u32;
    register_insert("clap:lumit.test.refuse", Some(4), 0);
    let source = tone(rate, 1.0, 0.8);
    let mut one = job("a.mp4", 0.0, 1.0, 0.0);
    one.chain = Some(chain_of(vec![insert_instance(
        "clap:lumit.test.refuse",
        Property::fixed(1.0),
    )]));
    let baked = baked(&[(source, one)], rate, 1.0);

    let block = lumit_core::fx::audio_chain::AUDIO_BLOCK_FRAMES;
    let inside = (4 * block + block / 2) * 2;
    assert!(
        (baked[inside] - 0.8).abs() < 1e-6,
        "the refused block is the input, dry: got {}",
        baked[inside]
    );
    assert!(
        (baked[0] - 0.5).abs() < 1e-6,
        "and every other block is still processed"
    );
    let worst = baked
        .chunks_exact(2)
        .zip(baked.chunks_exact(2).skip(1))
        .map(|(a, b)| (b[0] - a[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        worst < 0.02,
        "the splice must not click: worst step {worst}"
    );
}

/// **Latency is compensated by placing the sound earlier** (the note's §7
/// plan 3): a plugin that answers N frames late has its run put down N
/// frames sooner and made N frames longer, so the wet lands where the dry
/// did and a lookahead limiter simply works.
#[test]
fn a_latent_plugin_is_placed_earlier_by_exactly_its_latency() {
    let rate = 48_000u32;
    let latency = 128u32;
    register_insert("clap:lumit.test.latent", None, latency);
    let source = tone(rate, 2.0, 0.2);
    let mut one = job("a.mp4", 1.0, 2.0, 0.0);
    one.chain = Some(chain_of(vec![insert_instance(
        "clap:lumit.test.latent",
        Property::fixed(1.0),
    )]));
    let mut decoded = HashMap::new();
    decoded.insert(one.item, source);
    let (plan, _) = plan_of(&[one], &decoded, rate, 3.0);

    let clip = &plan.clips[0];
    assert_eq!(
        clip.start_frame,
        i64::from(rate) - i64::from(latency),
        "placed earlier by the chain's own delay"
    );
    assert_eq!(
        clip.src_start, 0,
        "the processed run starts at its own zero"
    );
    assert_eq!(
        clip.len,
        rate as usize + latency as usize,
        "and is long enough for the delayed tail to come out"
    );
}

// ------------------------------------- the clip's rack and its fades --

/// A clip fade of `head`/`tail` seconds with the two shapes on it.
fn clip_fade(
    start_s: f64,
    head_s: f64,
    head_shape: lumit_core::sequence::FadeShape,
    end_s: f64,
    tail_s: f64,
    tail_shape: lumit_core::sequence::FadeShape,
) -> lumit_render::mix::jobs::ClipFade {
    lumit_render::mix::jobs::ClipFade {
        start_s,
        head_s,
        head_shape,
        end_s,
        tail_s,
        tail_shape,
        gain_db: 0.0,
    }
}

/// **Preview is export through a shaped crossfade and a clip effect**
/// (plan 3): two clips over one join, each with its own stored shape and
/// one of them carrying a rack of its own, mix to the same samples in the
/// live plan and in the bake.
#[test]
fn a_shaped_crossfade_with_a_clip_effect_mixes_the_same_in_both_paths() {
    use lumit_core::sequence::FadeShape;
    let rate = 48_000u32;
    register_insert("clap:lumit.test.crossfade", None, 0);
    // Clip A runs 0..1 and goes out over its last half second; clip B runs
    // 0.5..1.5 and comes in over its first, through its own plugin.
    let a_source = tone(rate, 1.0, 0.4);
    let b_source = tone(rate, 1.5, 0.8);
    let mut a = job("a.mp4", 0.0, 1.0, 0.0);
    a.fade = Some(clip_fade(
        0.0,
        0.0,
        FadeShape::Fast,
        1.0,
        0.5,
        FadeShape::Smooth,
    ));
    let mut b = job("b.mp4", 0.5, 1.5, 0.5);
    b.fade = Some(clip_fade(
        0.5,
        0.5,
        FadeShape::custom(0.2, 0.8, 0.6, 0.9),
        1.5,
        0.0,
        FadeShape::Fast,
    ));
    b.clip_chain = Some(chain_of(vec![insert_instance(
        "clap:lumit.test.crossfade",
        Property::fixed(1.0),
    )]));

    let mut decoded = HashMap::new();
    decoded.insert(a.item, Arc::clone(&a_source));
    decoded.insert(b.item, Arc::clone(&b_source));
    let (plan, _) = plan_of(&[a.clone(), b.clone()], &decoded, rate, 2.0);
    let baked = baked(&[(a_source, a), (b_source, b)], rate, 2.0);

    for i in 0..(2 * rate as usize) {
        let (l, r) = plan.frame_at(i);
        assert!(
            (l - baked[i * 2]).abs() < 1e-9 && (r - baked[i * 2 + 1]).abs() < 1e-9,
            "frame {i}: the live plan and the export disagree"
        );
    }
    // And both really happened: past the join B stands alone, held at the
    // plugin's ±0.5, and half way across it neither clip is at full level.
    let after = baked[(rate as usize * 6 / 5) * 2];
    assert!(
        (after - 0.5).abs() < 1e-5,
        "the clip's own plugin ran: expected 0.5, got {after}"
    );
    let middle = baked[(rate as usize * 3 / 4) * 2];
    assert!(middle > 0.0 && middle < 0.9, "a shaped join, got {middle}");
}

/// The signature catches every edit the Audio timeline can make that
/// changes the sound without moving a clip (plan 5): a fade's shape, a
/// clip's bypass, an effect added to a clip, and the row's own fx
/// switch.
#[test]
fn the_signature_tracks_the_fades_and_both_racks() {
    use lumit_core::sequence::FadeShape;
    let one = |fade, clip_chain, chain| {
        let mut j = job("a.mp4", 0.0, 5.0, 0.0);
        j.layer = Uuid::nil();
        j.fade = fade;
        j.clip_chain = clip_chain;
        j.chain = chain;
        vec![j]
    };
    let fast = Some(clip_fade(
        0.0,
        0.5,
        FadeShape::Fast,
        5.0,
        0.0,
        FadeShape::Fast,
    ));
    let sig = jobs_signature(&one(fast, None, None), 10.0, 0.0);

    let slow = Some(clip_fade(
        0.0,
        0.5,
        FadeShape::Slow,
        5.0,
        0.0,
        FadeShape::Fast,
    ));
    assert_ne!(
        sig,
        jobs_signature(&one(slow, None, None), 10.0, 0.0),
        "a shape edit that moves no clip still re-plans"
    );
    let longer = Some(clip_fade(
        0.0,
        0.75,
        FadeShape::Fast,
        5.0,
        0.0,
        FadeShape::Fast,
    ));
    assert_ne!(
        sig,
        jobs_signature(&one(longer, None, None), 10.0, 0.0),
        "and so does a longer fade"
    );

    let racked = chain_of(vec![insert_instance(
        "clap:lumit.test.signature",
        Property::fixed(1.0),
    )]);
    let with_clip_fx = jobs_signature(&one(fast, Some(Arc::clone(&racked)), None), 10.0, 0.0);
    assert_ne!(sig, with_clip_fx, "an effect added to a clip re-plans");
    assert_ne!(
        with_clip_fx,
        jobs_signature(&one(fast, None, None), 10.0, 0.0),
        "and the clip's bypass, which takes the chain away, re-plans back"
    );

    let with_row_fx = jobs_signature(&one(fast, None, Some(racked)), 10.0, 0.0);
    assert_ne!(sig, with_row_fx, "the row's fx switch re-plans too");
    assert_ne!(
        with_clip_fx, with_row_fx,
        "and the same stack on the two racks is not the same mix"
    );
}
