//! The VST3 host, against the same in-tree fixture wearing its other face
//! (docs/impl/audio-plugins.md §7 plans 1, 2, 6 and 7; K-707).
//!
//! These mirror [`crate::tests`] deliberately, assertion for assertion, because
//! the promise AP4 makes is that **nothing downstream of describe knows which
//! standard a plugin speaks**. A VST3 plugin has to land as the same schema
//! rows, play the same sample-exact block, round-trip the same opaque blob and
//! obey the same "properties win over stale state" rule — and the way to show
//! that is to ask it the same questions.
//!
//! Two things differ, and both are asserted rather than assumed: the **order of
//! actions** is VST3's own ([`VST3_HOST_ACTIONS`]), and every value crosses the
//! boundary **normalised**, so a plain number that comes back plain has been
//! through the controller's conversion twice.
//!
//! Every test that opens the bundle takes the same [`fixture_lock`] the CLAP
//! tests do: the fixture's logs are statics inside a loaded library, and though
//! the `.clap` copy and the `.vst3` copy are two loaded modules with two sets of
//! them, the environment variables and the search paths are one process's.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use lumit_aplug_testplug::{Kind, PARAM_GAIN, PARAM_KNOB, PARAM_SWEEP, STATE_ECHO_DEFAULT};
use lumit_core::fx::{EffectDef, ParamId, ParamKind};

use crate::abi::{Abi, AnyModule};
use crate::def::{AudioEffectDef, AudioHost, InstanceSetup, LocalHost};
use crate::describe::{describe, describe_module};
use crate::discover::{scan, scan_dir, ScanOptions};
use crate::process::{ParamEvent, INTERLEAVED_LEN};
use crate::schema::schema_of;
use crate::tests::{a_ramp, action_log_of, built_cdylib, fixture_lock, reset_log_of, skipped};
use crate::vst3::{hex_of, join_state, split_state, tuid_from_hex};
use crate::VST3_HOST_ACTIONS;

// ---------------------------------------------------------------- fixture --

/// Lay the fixture out as a `.vst3` bundle under `root`, and answer the bundle.
///
/// A bundle is a folder, not a file: the library lives at
/// `Contents/<architecture>/`, and finding it again is
/// [`crate::vst3::payload`]'s job. Building the folder here rather than reaching
/// for the legacy plain-DLL shape is the point — the folder is what a real
/// installer writes, and the shape the host has to walk.
pub(crate) fn a_bundle_in(root: &Path) -> Option<PathBuf> {
    let source = built_cdylib()?;
    let bundle = root.join("lumit-test.vst3");
    let inside = bundle.join("Contents").join(architecture());
    std::fs::create_dir_all(&inside).ok()?;
    std::fs::copy(&source, inside.join("lumit-test.vst3")).ok()?;
    Some(bundle)
}

/// The architecture folder a bundle on this platform is read from.
fn architecture() -> &'static str {
    if cfg!(target_os = "windows") {
        "x86_64-win"
    } else if cfg!(target_os = "macos") {
        "MacOS"
    } else {
        "x86_64-linux"
    }
}

/// The one `.vst3` this process loads, laid out once.
///
/// One path for the whole process, for the reason [`crate::tests::fixture`]
/// gives: the loader answers with the same module for the same file, so the
/// host's copy and the test's own handle share the statics the log lives in.
fn fixture() -> Option<&'static Path> {
    static FIXTURE: OnceLock<Option<(tempfile::TempDir, PathBuf)>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let dir = tempfile::tempdir().ok()?;
            let bundle = a_bundle_in(dir.path())?;
            Some((dir, bundle))
        })
        .as_ref()
        .map(|(_, path)| path.as_path())
}

/// The library inside the bundle — where the logs actually live.
fn loaded_binary() -> Option<PathBuf> {
    crate::vst3::payload(fixture()?)
}

/// The module, open.
fn open_module() -> Option<AnyModule> {
    AnyModule::open(fixture()?).ok()
}

/// One of the eight, by kind — a VST3 class id rather than a CLAP id string, so
/// it has to be read off the module the host just described.
fn class_of(module: &AnyModule, kind: Kind) -> Option<String> {
    module
        .entries()
        .iter()
        .find(|entry| entry.name == name_of(kind))
        .map(|entry| entry.id.clone())
}

/// The name a person would see, as the fixture spells it.
fn name_of(kind: Kind) -> String {
    String::from_utf8_lossy(kind.name())
        .trim_end_matches('\0')
        .to_string()
}

// -------------------------------------------------------------- discovery --

#[test]
fn a_scan_finds_a_bundle_as_a_bundle_and_not_as_a_binary() {
    let _guard = fixture_lock();
    let Some(bundle) = fixture() else {
        return skipped("a_scan_finds_a_bundle_as_a_bundle_and_not_as_a_binary");
    };
    let Some(dir) = bundle.parent() else {
        return;
    };
    let found = scan_dir(dir);
    assert_eq!(
        found,
        vec![bundle.to_path_buf()],
        "the folder is what a scan hands back — which binary inside it belongs \
         to this machine is one question, answered in one place"
    );
}

#[test]
fn a_scan_offers_the_vst3_effects_and_reports_the_refusals() {
    let _guard = fixture_lock();
    let Some(bundle) = fixture() else {
        return skipped("a_scan_offers_the_vst3_effects_and_reports_the_refusals");
    };
    let Some(dir) = bundle.parent() else {
        return;
    };
    let outcome = scan(&ScanOptions {
        paths: vec![dir.to_path_buf()],
        ..ScanOptions::default()
    });

    assert_eq!(outcome.found.len(), 7, "seven of the eight are effects");
    assert!(
        outcome
            .found
            .iter()
            .all(|plugin| plugin.match_name.starts_with("vst3:")),
        "a VST3 effect is named for its own standard: {:?}",
        outcome
            .found
            .iter()
            .map(|plugin| plugin.match_name.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        outcome
            .found
            .iter()
            .any(|plugin| plugin.label == name_of(Kind::Gain) && plugin.vendor == "Lumit"),
        "and it carries the name and the vendor the class declared"
    );
    assert!(
        outcome
            .skipped
            .iter()
            .any(|line| line.contains("no audio input")),
        "the instrument's refusal is one calm line: {:?}",
        outcome.skipped
    );
}

// --------------------------------------------------------------- describe --

#[test]
fn a_bundle_lists_only_the_classes_that_make_sound() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_bundle_lists_only_the_classes_that_make_sound");
    };
    assert_eq!(module.abi(), Abi::Vst3);
    assert_eq!(
        module.entries().len(),
        8,
        "the eight controllers are furniture, not effects"
    );
    let first = &module.entries()[0];
    assert_eq!(first.name, name_of(Kind::Gain));
    assert_eq!(first.vendor, "Lumit");
    assert_eq!(
        first.id.len(),
        32,
        "a VST3 plugin is named by its class id, spelled in hex: {}",
        first.id
    );
    assert!(first.features.contains(&"Fx".to_owned()));
}

#[test]
fn an_instrument_is_refused_with_a_reason_in_vst3_too() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("an_instrument_is_refused_with_a_reason_in_vst3_too");
    };
    let Some(instrument) = class_of(&module, Kind::Instrument) else {
        return;
    };
    let report = describe_module(&module);
    let refusal = report
        .rejected
        .iter()
        .find(|refusal| refusal.id == instrument)
        .expect("the instrument should be refused");
    assert!(
        refusal.reason.contains("no audio input"),
        "the reason names the fact, in the same words CLAP's does: {}",
        refusal.reason
    );
    assert_eq!(report.described.len(), 7);
}

#[test]
fn a_described_vst3_plugin_lands_as_ordinary_properties() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_described_vst3_plugin_lands_as_ordinary_properties");
    };
    let Some(gain) = class_of(&module, Kind::Gain) else {
        return;
    };
    let descriptor = describe(&module, &gain).expect("the gain plugin is an effect");
    assert_eq!(descriptor.abi, Abi::Vst3);

    let schema = schema_of(&descriptor).expect("one row");
    let row = schema.params.first().expect("one row");
    assert_eq!(row.id, format!("p{PARAM_GAIN}"));
    assert_eq!(row.label, "Gain");
    // The range is the **plain** one, not nought to one: a person keyframes the
    // number they read, and the normalising happens at the boundary.
    assert!(
        matches!(row.kind, ParamKind::Slider { default, range } if default == 1.0 && range == (0.0, 4.0)),
        "a closed VST3 range is a slider in plain units: {:?}",
        row.kind
    );
    assert_eq!(schema.match_name, format!("vst3:{gain}"));

    let def = AudioEffectDef::new(&descriptor, Box::leak(Box::new(schema)), module.path());
    assert_eq!(def.plugin_param(ParamId::new("p1")), Some(PARAM_GAIN));
    assert_eq!(def.defaults(), &[(PARAM_GAIN, 1.0)]);
    assert!(!def.is_image_op(), "an audio effect touches no picture");
}

#[test]
fn only_automatable_visible_vst3_parameters_become_rows() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("only_automatable_visible_vst3_parameters_become_rows");
    };
    let Some(echo) = class_of(&module, Kind::ParamEcho) else {
        return;
    };
    let descriptor = describe(&module, &echo).expect("the echo plugin is an effect");
    assert_eq!(descriptor.params.len(), 3, "it declares three parameters");
    let schema = schema_of(&descriptor).expect("its rows are distinct");
    let ids: Vec<&str> = schema.params.iter().map(|row| row.id).collect();
    assert_eq!(
        ids,
        vec![format!("p{PARAM_SWEEP}")],
        "the hidden and the read-only parameters get no row, by the same one \
         rule CLAP's are judged by"
    );
}

// -------------------------------------------------- the order of actions --

#[test]
fn the_vst3_order_of_actions_is_the_one_written_down() {
    let _guard = fixture_lock();
    let (Some(bundle), Some(binary)) = (fixture(), loaded_binary()) else {
        return skipped("the_vst3_order_of_actions_is_the_one_written_down");
    };
    // The bundle has already been enumerated, so reset and enumerate again: the
    // log has to start at the factory.
    reset_log_of(&binary);
    let Ok(module) = AnyModule::open(bundle) else {
        return skipped("the_vst3_order_of_actions_is_the_one_written_down");
    };
    let _ = describe_module(&module);

    let Some(reporter) = class_of(&module, Kind::Reporter) else {
        return;
    };
    let setup = InstanceSetup {
        plugin_id: reporter,
        state: Some(join_state(&[1, 2, 3, 4], &[9, 9])),
        params: vec![(PARAM_KNOB, 0.75)],
        offline: false,
    };
    let host = LocalHost::open(&module, &setup).expect("the reporter opens");
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    host.process(&a_ramp(), &mut output, &[], 0)
        .expect("one block");
    drop(host);

    assert_eq!(action_log_of(&binary), VST3_HOST_ACTIONS.to_vec());
}

// -------------------------------------------------------------- the sound --

#[test]
fn a_vst3_gain_plugin_multiplies_every_sample_exactly() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_vst3_gain_plugin_multiplies_every_sample_exactly");
    };
    let Some(gain) = class_of(&module, Kind::Gain) else {
        return;
    };
    let setup = InstanceSetup {
        plugin_id: gain,
        params: vec![(PARAM_GAIN, 0.5)],
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &setup).expect("the gain plugin opens");

    let input = a_ramp();
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    host.process(&input, &mut output, &[], 0)
        .expect("one block");

    // A half of a nought-to-four range is an eighth normalised, and both are
    // exact in binary — so this is sample for sample, and it is also the
    // normalised round trip: the value left as plain, crossed as normalised, and
    // was used as plain again.
    let expected: Vec<f32> = input.iter().map(|sample| sample * 0.5).collect();
    assert_eq!(output, expected);
}

#[test]
fn a_vst3_parameter_event_inside_a_block_reaches_the_plugin() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_vst3_parameter_event_inside_a_block_reaches_the_plugin");
    };
    let Some(gain) = class_of(&module, Kind::Gain) else {
        return;
    };
    let setup = InstanceSetup {
        plugin_id: gain,
        params: vec![(PARAM_GAIN, 1.0)],
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &setup).expect("the gain plugin opens");

    let input = vec![1.0f32; INTERLEAVED_LEN];
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    let events = [ParamEvent {
        time: 0,
        id: PARAM_GAIN,
        value: 2.0,
    }];
    host.process(&input, &mut output, &events, 0)
        .expect("one block");
    assert!(
        output.iter().all(|sample| *sample == 2.0),
        "the block's own value beat the project's baseline: {:?}",
        &output[..4]
    );
}

#[test]
fn latency_is_read_off_the_live_vst3_plugin() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("latency_is_read_off_the_live_vst3_plugin");
    };
    let (Some(latency), Some(gain)) = (
        class_of(&module, Kind::Latency),
        class_of(&module, Kind::Gain),
    ) else {
        return;
    };
    let host = LocalHost::open(
        &module,
        &InstanceSetup {
            plugin_id: latency,
            ..InstanceSetup::default()
        },
    )
    .expect("the latency plugin opens");
    assert_eq!(host.latency(), lumit_aplug_testplug::LATENCY_DEFAULT);

    let host = LocalHost::open(
        &module,
        &InstanceSetup {
            plugin_id: gain,
            ..InstanceSetup::default()
        },
    )
    .expect("the gain plugin opens");
    assert_eq!(host.latency(), 0, "an effect with no delay reports none");
}

// -------------------------------------------------------------- the state --

#[test]
fn a_vst3_state_blob_round_trips_both_halves() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_vst3_state_blob_round_trips_both_halves");
    };
    let Some(echo) = class_of(&module, Kind::StateEcho) else {
        return;
    };
    // Two halves, deliberately different lengths, so a blob that came back with
    // the split in the wrong place could not pass.
    let processor: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let controller: Vec<u8> = b"the controller's own memory".to_vec();
    let blob = join_state(&processor, &controller);

    let host = LocalHost::open(
        &module,
        &InstanceSetup {
            plugin_id: echo.clone(),
            state: Some(blob.clone()),
            ..InstanceSetup::default()
        },
    )
    .expect("the state plugin opens");
    assert_eq!(
        host.save().expect("it saves"),
        blob,
        "both halves came back, byte for byte, in the order they went out"
    );
    assert_eq!(host.warning(), None, "nothing went wrong bringing it up");

    // And with nothing to load, what it saves is its own answer, not silence.
    let host = LocalHost::open(
        &module,
        &InstanceSetup {
            plugin_id: echo,
            ..InstanceSetup::default()
        },
    )
    .expect("it opens without a blob");
    assert_eq!(
        split_state(&host.save().expect("it saves")).0,
        STATE_ECHO_DEFAULT
    );
}

#[test]
fn properties_win_over_a_stale_vst3_state() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("properties_win_over_a_stale_vst3_state");
    };
    let Some(gain) = class_of(&module, Kind::Gain) else {
        return;
    };
    // The blob says four; the project says two. The project is this year's
    // answer, and a keyframed gain must not revert to a preset's — which for
    // VST3 means the baseline the host lays into every block has to beat what
    // the component read out of its own state.
    let setup = InstanceSetup {
        plugin_id: gain,
        state: Some(join_state(&4.0f64.to_le_bytes(), &[])),
        params: vec![(PARAM_GAIN, 2.0)],
        offline: false,
    };
    let host = LocalHost::open(&module, &setup).expect("the gain plugin opens");

    let input = vec![1.0f32; INTERLEAVED_LEN];
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    host.process(&input, &mut output, &[], 0)
        .expect("one block");
    assert!(
        output.iter().all(|sample| *sample == 2.0),
        "the property won: {:?}",
        &output[..4]
    );
}

#[test]
fn the_two_halves_of_a_state_survive_being_one_blob() {
    // A pure round trip, with no plugin in it: the length prefix is the only
    // thing standing between two blobs and one, and an off-by-one there would
    // hand a plugin somebody else's bytes.
    for (processor, controller) in [
        (vec![], vec![]),
        (vec![1u8], vec![]),
        (vec![], vec![2u8, 3]),
        (vec![7u8; 300], vec![9u8; 5]),
    ] {
        let blob = join_state(&processor, &controller);
        assert_eq!(
            split_state(&blob),
            (processor.as_slice(), controller.as_slice())
        );
    }
    // A blob too short to hold a length is all the processor's, which is what a
    // blob written by another program would be.
    assert_eq!(split_state(&[1, 2]), (&[1u8, 2][..], &[][..]));
}

#[test]
fn a_class_id_survives_the_hex_it_is_named_by() {
    let cid = crate::vst3::class_id(0x4C554D49, 0x54455354, 0x50524F43, 3);
    let text = hex_of(&cid);
    assert_eq!(text.len(), 32);
    assert_eq!(tuid_from_hex(&text), Some(cid));
    assert_eq!(tuid_from_hex("not a class id"), None);
    assert_eq!(tuid_from_hex(&text[..30]), None);
}

// --------------------------------------------------------- the automation --

#[test]
fn a_vst3_param_sweep_arrives_as_sorted_per_block_points() {
    let _guard = fixture_lock();
    let (Some(module), Some(binary)) = (open_module(), loaded_binary()) else {
        return skipped("a_vst3_param_sweep_arrives_as_sorted_per_block_points");
    };
    let Some(echo) = class_of(&module, Kind::ParamEcho) else {
        return;
    };
    let host = LocalHost::open(
        &module,
        &InstanceSetup {
            plugin_id: echo,
            ..InstanceSetup::default()
        },
    )
    .expect("the echo plugin opens");
    reset_log_of(&binary);

    let input = vec![0.0f32; INTERLEAVED_LEN];
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    for block in 0..3u32 {
        // Deliberately out of order. A VST3 queue is read front to back, so the
        // boundary sorts for the same reason CLAP's does and no caller has to
        // remember.
        let events = [
            ParamEvent {
                time: 256,
                id: PARAM_SWEEP,
                value: f64::from(block) * 0.1 + 0.03,
            },
            ParamEvent {
                time: 0,
                id: PARAM_SWEEP,
                value: f64::from(block) * 0.1 + 0.01,
            },
            ParamEvent {
                time: 128,
                id: PARAM_SWEEP,
                value: f64::from(block) * 0.1 + 0.02,
            },
        ];
        host.process(&input, &mut output, &events, i64::from(block) * 512)
            .expect("one block");
    }

    let seen = crate::tests::read_export(&binary, b"LumitTestPlugParamLog\0");
    assert_eq!(
        seen.len(),
        9,
        "three points a block, three blocks, and nothing extra — the sweep's own \
         row is the only one the baseline could have added: {seen:?}"
    );
    assert_eq!(
        seen,
        vec![
            format!("0:0:{PARAM_SWEEP}:0.010000"),
            format!("0:128:{PARAM_SWEEP}:0.020000"),
            format!("0:256:{PARAM_SWEEP}:0.030000"),
            format!("1:0:{PARAM_SWEEP}:0.110000"),
            format!("1:128:{PARAM_SWEEP}:0.120000"),
            format!("1:256:{PARAM_SWEEP}:0.130000"),
            format!("2:0:{PARAM_SWEEP}:0.210000"),
            format!("2:128:{PARAM_SWEEP}:0.220000"),
            format!("2:256:{PARAM_SWEEP}:0.230000"),
        ]
    );
}
