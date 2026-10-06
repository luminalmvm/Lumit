//! The VST3 host, against the same in-tree fixture wearing its other face
//! (docs/impl/audio-plugins.md §7 plans 1, 2, 6 and 7).
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
use crate::discover::{scan, ScanOptions};
use crate::process::{ParamEvent, INTERLEAVED_LEN};
use crate::schema::schema_of;
use crate::tests::{a_ramp, action_log_of, built_cdylib, fixture_lock, reset_log_of, skipped};
use crate::vst3::{join_state, split_state};
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
        matches!(row.kind, ParamKind::Slider { default, range, .. } if default == 1.0 && range == (0.0, 4.0)),
        "a closed VST3 range is a slider in plain units: {:?}",
        row.kind
    );
    assert_eq!(schema.match_name, format!("vst3:{gain}"));

    let def = AudioEffectDef::new(&descriptor, Box::leak(Box::new(schema)), module.path());
    assert_eq!(def.plugin_param(ParamId::new("p1")), Some(PARAM_GAIN));
    assert_eq!(def.defaults(), &[(PARAM_GAIN, 1.0)]);
    assert!(!def.is_image_op(), "an audio effect touches no picture");
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
        rate: 48_000,
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
