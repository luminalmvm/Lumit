//! What the broker is for: the plugin dies and the mix does not
//! (docs/impl/audio-plugins.md §7 plan 5, the Gate-4 shape).
//!
//! These are the tests that can only be written against a real second process —
//! a plugin that aborts partway through a block, one that never comes back, and
//! one the user switches off while it is playing. The plugin is told to
//! misbehave through the broker's environment, because a plugin in another
//! process cannot be reached any other way.
//!
//! They live in this crate rather than in `lumit-aplug` for one flat Cargo
//! reason: `CARGO_BIN_EXE_lumit-aplug-broker` exists only inside the package
//! that owns the binary, and Cargo does not build a dependency's binaries.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lumit_aplug::def::{BlockJob, BrokerHost};
use lumit_aplug::ipc::proto::Bring;
use lumit_aplug::ipc::ring::Ring;
use lumit_aplug::{
    nothing_disabled, scan_brokered, AudioHost, Broker, BrokerConfig, BrokerError, DisableList,
    InstanceSetup, INTERLEAVED_LEN,
};
use lumit_aplug_testplug::{Kind, CRASH_ON_BLOCK_ENV, HANG_ENV, PARAM_GAIN};

// ------------------------------------------------------------ the scaffold --

/// The test plugin's file name on this platform.
fn cdylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_aplug_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_aplug_testplug.dylib"
    } else {
        "liblumit_aplug_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
fn built_cdylib() -> Option<PathBuf> {
    let name = cdylib_name();
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..3 {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        let in_deps = dir.join("deps").join(name);
        if in_deps.is_file() {
            return Some(in_deps);
        }
        dir = dir.parent()?;
    }
    None
}

/// Lay the test plugin out as a `.clap` file in `root`.
///
/// Unlike the in-process tests, each case gets its **own copy**: the plugin's
/// logs are statics inside a loaded library, and the library here is loaded in
/// another process that is about to be killed, so there is nothing to share.
fn a_module_in(root: &Path) -> Option<PathBuf> {
    let source = built_cdylib()?;
    let target = root.join("lumit-test.clap");
    std::fs::copy(&source, &target).ok()?;
    Some(target)
}

/// Say why a test did nothing, by name, so a skip is never silent.
fn skipped(test: &str) {
    eprintln!(
        "{test}: skipped — {} was not found in the target directory. \
         Build it first: cargo build -p lumit-aplug-testplug",
        cdylib_name()
    );
}

/// One of the eight, by kind.
fn plugin_id(kind: Kind) -> String {
    String::from_utf8_lossy(kind.id())
        .trim_end_matches('\0')
        .to_string()
}

/// A broker over the test module, with whatever environment the case wants the
/// plugin to misbehave under.
fn a_broker(root: &Path, env: &[(&str, &str)], disabled: &DisableList) -> Option<Broker> {
    let module = a_module_in(root)?;
    let mut config = BrokerConfig::new(module);
    // The shipped floor under a block deadline is one block period, eleven
    // milliseconds. That is too tight to tell a hung plugin from a slow build
    // machine, so the floor is raised — which is exactly what a quirks-table
    // entry writes, and the point: the override is the mechanism, not a test
    // hook. A hang then costs 200 ms three times, and nothing else waits.
    config.quirks.block_floor = Duration::from_millis(200);
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-aplug-broker")));
    config.disabled = Arc::clone(disabled);
    config.env = env
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();

    let mut broker = Broker::spawn(config).expect("a broker");
    broker.describe().expect("a description");
    Some(broker)
}

/// The gain plugin, brought up in a broker, with its multiplier set.
fn a_gain_host(broker: Arc<Mutex<Broker>>, gain: f64) -> BrokerHost {
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Gain),
        params: vec![(PARAM_GAIN, gain)],
        ..InstanceSetup::default()
    };
    BrokerHost::open(broker, &setup).expect("an instance")
}

/// A block every sample of which is the same value, so a test can say what came
/// back in one number.
fn a_flat_block(value: f32) -> Vec<f32> {
    vec![value; INTERLEAVED_LEN]
}

/// One batch of `count` identical blocks, and the answers to it.
fn a_batch(host: &BrokerHost, count: usize, value: f32) -> (Vec<f32>, Vec<Option<String>>) {
    let input = a_flat_block(value);
    let jobs: Vec<BlockJob<'_>> = (0..count)
        .map(|index| BlockJob::new(&input, &[], (index * 512) as i64))
        .collect();
    let mut outputs = vec![0.0f32; count * INTERLEAVED_LEN];
    let answers = host.process_batch(&jobs, &mut outputs);
    let failures = answers
        .into_iter()
        .map(|answer| answer.err().map(|error| error.to_string()))
        .collect();
    (outputs, failures)
}

/// The first sample of block `index` in a batch's output.
fn first_of(outputs: &[f32], index: usize) -> f32 {
    outputs[index * INTERLEAVED_LEN]
}

// ---------------------------------------------------------------- the tests --

#[test]
fn a_module_describes_itself_without_ever_being_loaded_here() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(broker) = a_broker(root.path(), &[], &nothing_disabled()) else {
        return skipped("a_module_describes_itself_without_ever_being_loaded_here");
    };
    // Seven of the eight came back across the pipe, with nothing of the plugin
    // in this process at all.
    assert_eq!(broker.descriptors().len(), 7);
    assert!(broker
        .descriptors()
        .iter()
        .any(|descriptor| descriptor.id == plugin_id(Kind::Gain)));
    // And the eighth is one calm line rather than a failure.
    assert!(
        broker
            .rejected()
            .iter()
            .any(|refusal| refusal.id == plugin_id(Kind::Instrument)
                && refusal.reason.contains("no audio input")),
        "the instrument's refusal should cross too: {:?}",
        broker.rejected()
    );
    assert!(!broker.is_disabled());
}

#[test]
fn a_brokered_scan_offers_the_effects_without_opening_a_module_here() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    if a_module_in(root.path()).is_none() {
        return skipped("a_brokered_scan_offers_the_effects_without_opening_a_module_here");
    }
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_lumit-aplug-broker"));
    let outcome = scan_brokered(
        &[root.path().to_path_buf()],
        &nothing_disabled(),
        Some(&exe),
    );

    let names: Vec<&str> = outcome
        .found
        .iter()
        .map(|plugin| plugin.match_name.as_str())
        .collect();
    assert!(
        names.contains(&"clap:com.lumit.aplug.testplug.gain"),
        "the gain effect should be offered: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name.contains("instrument")),
        "an instrument is not an effect: {names:?}"
    );
    assert!(
        outcome
            .skipped
            .iter()
            .any(|line| line.contains("instrument") && line.contains("no audio input")),
        "and its refusal is one calm line: {:?}",
        outcome.skipped
    );
}

#[test]
fn a_switched_off_plugin_is_never_described_in_the_broker_either() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let disabled: DisableList = Arc::new(Mutex::new([plugin_id(Kind::Gain)].into_iter().collect()));
    let Some(broker) = a_broker(root.path(), &[], &disabled) else {
        return skipped("a_switched_off_plugin_is_never_described_in_the_broker_either");
    };
    assert!(
        !broker
            .descriptors()
            .iter()
            .any(|descriptor| descriptor.id == plugin_id(Kind::Gain)),
        "the list travels with the question, so the plugin is never created"
    );
    assert_eq!(broker.descriptors().len(), 6);
}

#[test]
fn a_crash_costs_exactly_one_block_and_the_blocks_after_it_flow() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    // Block nought plays, block one aborts the process. After the restart the
    // plugin is a fresh copy counting from nought again, so block two is its
    // block nought and plays.
    let Some(broker) = a_broker(
        root.path(),
        &[(CRASH_ON_BLOCK_ENV, "1")],
        &nothing_disabled(),
    ) else {
        return skipped("a_crash_costs_exactly_one_block_and_the_blocks_after_it_flow");
    };
    let broker = Arc::new(Mutex::new(broker));
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Crash),
        ..InstanceSetup::default()
    };
    let host = BrokerHost::open(Arc::clone(&broker), &setup).expect("an instance");

    let (outputs, failures) = a_batch(&host, 3, 0.25);

    assert_eq!(
        failures.iter().filter(|failure| failure.is_some()).count(),
        1,
        "a dying plugin costs exactly one block: {failures:?}"
    );
    assert!(failures[1].is_some(), "and it is the block it died in");
    assert!(
        (first_of(&outputs, 0) - 0.25).abs() < 1e-6,
        "the block before is the plugin's own work"
    );
    assert!(
        (first_of(&outputs, 2) - 0.25).abs() < 1e-6,
        "and so is the block after, through a process that did not exist a \
         moment ago, with an instance nobody rebuilt by hand"
    );

    let held = broker.lock().expect("the broker");
    assert!(!held.is_disabled(), "one crash is not three");
    assert!(held.restarts() >= 1, "the broker must have been restarted");
}

#[test]
fn a_hang_trips_the_deadline_and_the_third_strike_disables_the_plugin() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(broker) = a_broker(root.path(), &[(HANG_ENV, "1")], &nothing_disabled()) else {
        return skipped("a_hang_trips_the_deadline_and_the_third_strike_disables_the_plugin");
    };
    let broker = Arc::new(Mutex::new(broker));
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Hang),
        ..InstanceSetup::default()
    };
    let host = BrokerHost::open(Arc::clone(&broker), &setup).expect("an instance");

    // Three blocks, none of which ever comes back: the deadline fires each
    // time and each is a strike.
    let (_, failures) = a_batch(&host, 3, 0.75);
    assert!(
        failures.iter().all(Option::is_some),
        "a block that never came back is failed, and the caller ships it dry: \
         {failures:?}"
    );

    let restarts = {
        let held = broker.lock().expect("the broker");
        assert!(
            held.is_disabled(),
            "three consecutive failures put the plugin away for the session"
        );
        held.restarts()
    };

    // And it stays away: the next batch does not start another process.
    let (_, again) = a_batch(&host, 2, 0.75);
    assert!(again.iter().all(Option::is_some));
    assert_eq!(
        broker.lock().expect("the broker").restarts(),
        restarts,
        "a disabled plugin is not retried"
    );
}

#[test]
fn a_plugin_switched_off_mid_session_is_skipped_on_the_next_batch() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let disabled = nothing_disabled();
    let Some(broker) = a_broker(root.path(), &[], &disabled) else {
        return skipped("a_plugin_switched_off_mid_session_is_skipped_on_the_next_batch");
    };
    let broker = Arc::new(Mutex::new(broker));
    let host = a_gain_host(Arc::clone(&broker), 0.5);

    let (outputs, failures) = a_batch(&host, 2, 1.0);
    assert!(failures.iter().all(Option::is_none), "{failures:?}");
    assert!((first_of(&outputs, 0) - 0.5).abs() < 1e-6);

    // The user unticks it while the mix is playing.
    disabled
        .lock()
        .expect("the list")
        .insert(plugin_id(Kind::Gain));

    let (outputs, failures) = a_batch(&host, 2, 1.0);
    assert!(
        failures.iter().all(Option::is_some),
        "the next batch reads the list and skips the plugin: {failures:?}"
    );
    assert!(
        failures[0]
            .as_deref()
            .is_some_and(|why| why.contains("switched off")),
        "and it says which kind of skip it was: {failures:?}"
    );
    assert_eq!(
        first_of(&outputs, 0),
        0.0,
        "nothing was played, so nothing was written"
    );
    assert!(
        !broker.lock().expect("the broker").is_disabled(),
        "switching a plugin off is not a strike against it"
    );
}

#[test]
fn a_block_crosses_the_ring_unchanged() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let path = root.path().join("ring");
    let mut written = Ring::create(&path).expect("a ring");

    // Every sample different, so a slot written half way or a channel written
    // twice cannot pass.
    let block: Vec<f32> = (0..INTERLEAVED_LEN)
        .map(|index| index as f32 / 1024.0)
        .collect();
    let header = written.write_block(3, &block).expect("written");
    assert_eq!(header.samples as usize, INTERLEAVED_LEN);

    // Read through a *second* mapping of the same block, which is what the
    // broker process has.
    let read = Ring::open(written.spec()).expect("the same ring");
    let mut back = vec![0.0f32; INTERLEAVED_LEN];
    assert_eq!(read.read_block(3, &mut back).expect("read"), block.len());
    assert_eq!(back, block, "the samples cross whole");

    // A slot nobody wrote is empty rather than plausible.
    assert!(read.read_block(1, &mut back).is_err());

    // And a short block leaves silence where the sound ran out rather than the
    // previous block's tail.
    let mut short = written.write_block(3, &[1.0, 1.0, 1.0, 1.0]);
    assert!(short.is_ok());
    short = written.write_block(4, &[2.0; 8]);
    assert!(short.is_ok());
    assert_eq!(read.read_block(3, &mut back).expect("read"), 4);
    assert_eq!(back[0], 1.0);
    assert_eq!(back[4], 0.0);
}

#[test]
fn a_broker_that_speaks_another_protocol_is_refused() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(module) = a_module_in(root.path()) else {
        return skipped("a_broker_that_speaks_another_protocol_is_refused");
    };
    let mut config = BrokerConfig::new(module);
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-aplug-broker")));
    config.env = vec![("LUMIT_APLUG_BROKER_PROTOCOL".to_owned(), "99".to_owned())];

    match Broker::spawn(config) {
        Err(BrokerError::ProtocolMismatch { theirs, ours }) => {
            assert_eq!(theirs, 99);
            assert_eq!(ours, lumit_aplug::ipc::proto::PROTOCOL_VERSION);
        }
        Err(other) => panic!("expected a version refusal, got {other}"),
        Ok(_) => panic!("a broker of another version must be refused, not believed"),
    }
}

#[test]
fn sound_and_state_cross_the_boundary_intact() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(broker) = a_broker(root.path(), &[], &nothing_disabled()) else {
        return skipped("sound_and_state_cross_the_boundary_intact");
    };
    let broker = Arc::new(Mutex::new(broker));
    let host = a_gain_host(Arc::clone(&broker), 0.5);

    // Halving is exact in binary, so this is sample for sample and not
    // approximate — which is also what proves the ring, the de-interleave and
    // the re-interleave all put every sample back where it came from.
    let input: Vec<f32> = (0..INTERLEAVED_LEN)
        .map(|index| index as f32 / INTERLEAVED_LEN as f32)
        .collect();
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    host.process(&input, &mut output, &[], 0).expect("a block");
    let expected: Vec<f32> = input.iter().map(|sample| sample * 0.5).collect();
    assert_eq!(output, expected);

    // A blob handed over comes back byte for byte, having crossed twice.
    let blob: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let echo = InstanceSetup {
        plugin_id: plugin_id(Kind::StateEcho),
        state: Some(blob.clone()),
        ..InstanceSetup::default()
    };
    let stateful = BrokerHost::open(Arc::clone(&broker), &echo).expect("an instance");
    assert_eq!(stateful.save().expect("it saves"), blob);
}

#[test]
fn a_handle_the_broker_never_minted_is_answered_rather_than_followed() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[], &nothing_disabled()) else {
        return skipped("a_handle_the_broker_never_minted_is_answered_rather_than_followed");
    };
    // A plain counter — which is what a host that did not mint handles would
    // send — names nothing, and saying so costs the plugin a strike rather than
    // the session.
    match broker.save(7) {
        Err(BrokerError::Refused(why)) => assert!(why.contains("no such plugin instance"), "{why}"),
        other => panic!("a forged handle must be refused: {other:?}"),
    }
    assert_eq!(broker.strikes(), 1);
    assert!(!broker.is_disabled());

    // And the broker is still there afterwards: a bad handle is an answer, not
    // a fault in the process.
    let created = broker
        .create_instance(Bring {
            plugin_id: plugin_id(Kind::Gain),
            ..Bring::default()
        })
        .expect("an instance");
    assert_eq!(broker.strikes(), 0, "a good action clears the count");
    broker.destroy(created.instance);
}
