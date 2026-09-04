//! The CLAP host, against the in-tree test plugin
//! (docs/impl/audio-plugins.md §7, plans 1, 2, 6 and 7).
//!
//! Every test that opens the module takes [`fixture_lock`] first. The test
//! plugin records what the host did in statics that live inside the **loaded
//! copy** of the library, which every test in this process shares — so two
//! tests running at once would read each other's calls.

use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use lumit_aplug_testplug::{Kind, PARAM_GAIN, PARAM_KNOB, PARAM_SWEEP, STATE_ECHO_DEFAULT};
use lumit_core::fx::{EffectDef, ParamId, ParamKind};

use crate::abi::AnyModule;
use crate::def::{AudioEffectDef, AudioHost, InstanceSetup, LocalHost};
use crate::describe::{describe, describe_module};
use crate::discover::{clap_search_paths, scan, scan_dir, search_paths, ScanOptions};
use crate::process::{Block, ParamEvent, BLOCK_FRAMES, INTERLEAVED_LEN};
use crate::schema::schema_of;
use crate::HOST_ACTIONS;

// ---------------------------------------------------------------- fixture --

/// Serialises every test that loads the module, because the plugin's logs are
/// one set of statics shared by the whole process.
pub(crate) fn fixture_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The test plugin's file name on this platform.
pub(crate) fn cdylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_aplug_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_aplug_testplug.dylib"
    } else {
        "liblumit_aplug_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
pub(crate) fn built_cdylib() -> Option<PathBuf> {
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

/// The one `.clap` this process loads, copied out once.
///
/// **One path, for the whole process**: the operating system's loader hands
/// back the same module for the same file, so the host's copy and the test's
/// own handle share the statics the log lives in. Two copies in two temporary
/// folders would be two modules and two empty logs.
fn fixture() -> Option<&'static Path> {
    static FIXTURE: OnceLock<Option<(tempfile::TempDir, PathBuf)>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let source = built_cdylib()?;
            let dir = tempfile::tempdir().ok()?;
            let target = dir.path().join("lumit-test.clap");
            std::fs::copy(&source, &target).ok()?;
            Some((dir, target))
        })
        .as_ref()
        .map(|(_, path)| path.as_path())
}

/// Say why a test did nothing, by name, so a skip is never silent.
pub(crate) fn skipped(test: &str) {
    eprintln!("{test}: the test plugin was not built, so nothing was checked");
}

/// The module, open.
fn open_module() -> Option<AnyModule> {
    AnyModule::open(fixture()?).ok()
}

/// One of the eight, by kind.
pub(crate) fn plugin_id(kind: Kind) -> String {
    String::from_utf8_lossy(kind.id())
        .trim_end_matches('\0')
        .to_string()
}

/// Read one of the test plugin's own exports, out of the copy loaded from
/// `path`.
///
/// The path matters: the loader keys a module by file, so the `.clap` copy and
/// the `.vst3` copy of the same library are two modules with two sets of
/// statics, and a test must read the one its host loaded.
pub(crate) fn read_export(path: &Path, symbol: &[u8]) -> Vec<String> {
    // SAFETY: the same file the host loaded; the loader answers with the same
    // module, and the symbol is one this crate's own test plugin exports.
    let library = match unsafe { libloading::Library::new(path) } {
        Ok(library) => library,
        Err(_) => return Vec::new(),
    };
    // SAFETY: the export's signature is the one declared in the test plugin.
    let read: libloading::Symbol<'_, unsafe extern "C" fn(*mut c_char, u32) -> u32> =
        match unsafe { library.get(symbol) } {
            Ok(symbol) => symbol,
            Err(_) => return Vec::new(),
        };
    let mut buffer = vec![0 as c_char; 16 * 1024];
    let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    // SAFETY: the buffer is writable for its own length.
    let written = unsafe { read(buffer.as_mut_ptr(), capacity) } as usize;
    if written == 0 || written >= buffer.len() {
        return Vec::new();
    }
    let bytes: Vec<u8> = buffer[..written].iter().map(|byte| *byte as u8).collect();
    String::from_utf8_lossy(&bytes)
        .split(',')
        .map(str::to_owned)
        .collect()
}

/// Every host call the reporter saw, in the copy loaded from `path`.
pub(crate) fn action_log_of(path: &Path) -> Vec<String> {
    read_export(path, b"LumitTestPlugLog\0")
}

/// Every host call the reporter saw.
fn action_log() -> Vec<String> {
    fixture().map(action_log_of).unwrap_or_default()
}

/// Every parameter event the echo plugin was sent.
fn param_log() -> Vec<String> {
    fixture().map_or_else(Vec::new, |path| {
        read_export(path, b"LumitTestPlugParamLog\0")
    })
}

/// Empty both logs in the copy loaded from `path`.
pub(crate) fn reset_log_of(path: &Path) {
    // SAFETY: as `read_export`.
    let Ok(library) = (unsafe { libloading::Library::new(path) }) else {
        return;
    };
    // SAFETY: as `read_export`.
    let reset: libloading::Symbol<'_, unsafe extern "C" fn()> =
        match unsafe { library.get(b"LumitTestPlugResetLog\0") } {
            Ok(symbol) => symbol,
            Err(_) => return,
        };
    // SAFETY: the export takes and returns nothing.
    unsafe { reset() };
}

/// Empty both logs.
fn reset_log() {
    if let Some(path) = fixture() {
        reset_log_of(path);
    }
}

/// A ramp, as Lumit carries sound: interleaved stereo, one whole block.
pub(crate) fn a_ramp() -> Vec<f32> {
    (0..INTERLEAVED_LEN)
        .map(|index| index as f32 / INTERLEAVED_LEN as f32)
        .collect()
}

// -------------------------------------------------------------- discovery --

#[test]
fn the_search_paths_are_the_standard_ones_plus_clap_path() {
    // Env vars are process-wide, so this borrows the same lock the module
    // tests use rather than racing them.
    let _guard = fixture_lock();
    let standard = clap_search_paths();
    #[cfg(target_os = "windows")]
    assert!(
        standard.len() >= 2 && standard.iter().all(|path| path.ends_with("CLAP")),
        "the two Windows folders both end in CLAP: {standard:?}"
    );

    // Not a drive letter: `split_paths` splits on `:` on Unix, so "Z:/elsewhere"
    // arrives as two paths there and the count below is one too many. The
    // temporary directory is one path on every platform this builds for.
    let extra = std::env::temp_dir().join("lumit-clap-elsewhere");
    std::env::set_var("CLAP_PATH", &extra);
    let widened = clap_search_paths();
    std::env::remove_var("CLAP_PATH");

    assert_eq!(
        widened.len(),
        standard.len() + 1,
        "CLAP_PATH adds to the standard folders, it never replaces them"
    );
    assert_eq!(widened.last(), Some(&extra));

    // And one scan looks in both standards' folders, in that order.
    let both = search_paths();
    assert!(
        both.len() > standard.len(),
        "a scan looks for VST3 as well as CLAP: {both:?}"
    );
    assert!(
        both.starts_with(&standard),
        "CLAP's folders are still there, and still first: {both:?}"
    );
}

#[test]
fn scan_dir_finds_clap_files_and_ignores_everything_else() {
    let Some(path) = fixture() else {
        return skipped("scan_dir_finds_clap_files_and_ignores_everything_else");
    };
    let Some(dir) = path.parent() else {
        return;
    };
    std::fs::write(dir.join("readme.txt"), b"not a plugin").ok();
    let found = scan_dir(dir);
    assert_eq!(found, vec![path.to_path_buf()]);
}

#[test]
fn a_scan_offers_the_effects_and_reports_the_refusals() {
    let _guard = fixture_lock();
    let Some(path) = fixture() else {
        return skipped("a_scan_offers_the_effects_and_reports_the_refusals");
    };
    let Some(dir) = path.parent() else {
        return;
    };
    let options = ScanOptions {
        paths: vec![dir.to_path_buf()],
        ..ScanOptions::default()
    };
    let outcome = scan(&options);

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
        "the instrument's refusal should be one calm line: {:?}",
        outcome.skipped
    );
}

#[test]
fn a_switched_off_plugin_is_never_described() {
    let _guard = fixture_lock();
    let Some(path) = fixture() else {
        return skipped("a_switched_off_plugin_is_never_described");
    };
    let Some(dir) = path.parent() else {
        return;
    };
    let options = ScanOptions {
        paths: vec![dir.to_path_buf()],
        disabled: [plugin_id(Kind::Gain)].into_iter().collect(),
    };
    let outcome = scan(&options);
    assert!(
        !outcome
            .found
            .iter()
            .any(|plugin| plugin.identifier == plugin_id(Kind::Gain)),
        "a plugin the user switched off must not become an effect"
    );
}

// --------------------------------------------------------------- describe --

#[test]
fn a_module_lists_every_plugin_in_it() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_module_lists_every_plugin_in_it");
    };
    assert_eq!(module.entries().len(), 8);
    assert_eq!(module.entries()[0].id, plugin_id(Kind::Gain));
    assert_eq!(module.entries()[0].vendor, "Lumit");
    assert!(module.entries()[0]
        .features
        .contains(&"audio-effect".to_owned()));
}

#[test]
fn an_instrument_is_refused_with_a_reason() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("an_instrument_is_refused_with_a_reason");
    };
    let report = describe_module(&module);
    let refusal = report
        .rejected
        .iter()
        .find(|refusal| refusal.id == plugin_id(Kind::Instrument))
        .expect("the instrument should be refused");
    assert!(
        refusal.reason.contains("no audio input"),
        "the reason names the fact: {}",
        refusal.reason
    );
    assert_eq!(report.described.len(), 7);
}

#[test]
fn only_automatable_visible_parameters_become_rows() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("only_automatable_visible_parameters_become_rows");
    };
    let descriptor =
        describe(&module, &plugin_id(Kind::ParamEcho)).expect("the echo plugin is an effect");
    assert_eq!(descriptor.params.len(), 3, "it declares three parameters");

    let schema = schema_of(&descriptor).expect("its rows are distinct");
    let ids: Vec<&str> = schema.params.iter().map(|row| row.id).collect();
    assert_eq!(
        ids,
        vec!["p7"],
        "the hidden and the read-only parameters get no row"
    );
    assert_eq!(schema.match_name, "clap:com.lumit.aplug.testplug.paramecho");
}

#[test]
fn a_row_is_named_by_the_plugins_own_parameter_id() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_row_is_named_by_the_plugins_own_parameter_id");
    };
    let descriptor = describe(&module, &plugin_id(Kind::Gain)).expect("the gain plugin");
    let schema = schema_of(&descriptor).expect("one row");
    let row = schema.params.first().expect("one row");
    assert_eq!(row.id, format!("p{PARAM_GAIN}"));
    assert_eq!(row.label, "Gain");
    // 0…4 is the whole of what the parameter is, which is a Slider and not a
    // Float with a typing box beyond its ends.
    assert!(
        matches!(row.kind, ParamKind::Slider { default, range } if default == 1.0 && range == (0.0, 4.0)),
        "a closed CLAP range is a slider: {:?}",
        row.kind
    );

    let def = AudioEffectDef::new(&descriptor, Box::leak(Box::new(schema)), module.path());
    assert_eq!(def.plugin_param(ParamId::new("p1")), Some(PARAM_GAIN));
    assert_eq!(def.defaults(), &[(PARAM_GAIN, 1.0)]);
    assert!(!def.is_image_op(), "an audio effect touches no picture");
}

// -------------------------------------------------- the order of actions --

#[test]
fn the_order_of_actions_is_the_one_written_down() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("the_order_of_actions_is_the_one_written_down");
    };
    // The module has already enumerated its plugins, so reset and enumerate
    // again: the log has to start at the factory.
    reset_log();
    let module = match AnyModule::open(module.path()) {
        Ok(module) => module,
        Err(_) => return skipped("the_order_of_actions_is_the_one_written_down"),
    };
    let _ = describe_module(&module);

    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Reporter),
        state: Some(vec![1, 2, 3, 4]),
        params: vec![(PARAM_KNOB, 0.75)],
        offline: false,
    };
    let host = LocalHost::open(&module, &setup).expect("the reporter opens");
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    host.process(&a_ramp(), &mut output, &[], 0)
        .expect("one block");
    drop(host);

    assert_eq!(action_log(), HOST_ACTIONS.to_vec());
}

// -------------------------------------------------------------- the sound --

#[test]
fn a_gain_plugin_multiplies_every_sample_exactly() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_gain_plugin_multiplies_every_sample_exactly");
    };
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Gain),
        params: vec![(PARAM_GAIN, 0.5)],
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &setup).expect("the gain plugin opens");

    let input = a_ramp();
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    host.process(&input, &mut output, &[], 0)
        .expect("one block");

    // Halving is exact in binary, so this is sample-for-sample and not
    // approximate — which is also what proves the de- and re-interleave put
    // every sample back where it came from.
    let expected: Vec<f32> = input.iter().map(|sample| sample * 0.5).collect();
    assert_eq!(output, expected);
}

#[test]
fn a_parameter_event_inside_a_block_reaches_the_plugin() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_parameter_event_inside_a_block_reaches_the_plugin");
    };
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Gain),
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
        "the event should have been read before the block was processed"
    );
}

#[test]
fn latency_is_read_off_the_live_plugin() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("latency_is_read_off_the_live_plugin");
    };
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Latency),
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &setup).expect("the latency plugin opens");
    assert_eq!(host.latency(), lumit_aplug_testplug::LATENCY_DEFAULT);

    let quiet = InstanceSetup {
        plugin_id: plugin_id(Kind::Gain),
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &quiet).expect("the gain plugin opens");
    assert_eq!(host.latency(), 0, "an effect with no delay reports none");
}

// -------------------------------------------------------------- the state --

#[test]
fn a_state_blob_round_trips_byte_identical() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_state_blob_round_trips_byte_identical");
    };
    let blob: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::StateEcho),
        state: Some(blob.clone()),
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &setup).expect("the state plugin opens");
    assert_eq!(host.save().expect("it saves"), blob);
    assert_eq!(host.warning(), None, "nothing went wrong bringing it up");

    // And with nothing to load, what it saves is its own answer, not silence.
    let fresh = InstanceSetup {
        plugin_id: plugin_id(Kind::StateEcho),
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &fresh).expect("it opens without a blob");
    assert_eq!(host.save().expect("it saves"), STATE_ECHO_DEFAULT);
}

#[test]
fn properties_win_over_a_stale_state() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("properties_win_over_a_stale_state");
    };
    // The blob says four; the project says two. The project is this year's
    // answer, and a keyframed gain must not revert to a preset's.
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Gain),
        state: Some(4.0f64.to_le_bytes().to_vec()),
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
fn a_plugin_that_saves_nothing_is_not_a_failure() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_plugin_that_saves_nothing_is_not_a_failure");
    };
    // Every one of the eight implements `state`, so the honest check here is
    // the other half of the rule: a blob handed to a plugin that refuses it
    // degrades to a warning rather than losing the effect.
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::Gain),
        state: Some(vec![]),
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &setup).expect("an empty blob does not stop it opening");
    assert_eq!(host.warning(), None);
}

// ---------------------------------------------------------- the automation --

#[test]
fn a_param_sweep_arrives_as_sorted_per_block_events() {
    let _guard = fixture_lock();
    let Some(module) = open_module() else {
        return skipped("a_param_sweep_arrives_as_sorted_per_block_events");
    };
    let setup = InstanceSetup {
        plugin_id: plugin_id(Kind::ParamEcho),
        ..InstanceSetup::default()
    };
    let host = LocalHost::open(&module, &setup).expect("the echo plugin opens");
    reset_log();

    let input = vec![0.0f32; INTERLEAVED_LEN];
    let mut output = vec![0.0f32; INTERLEAVED_LEN];
    for block in 0..3u32 {
        // Deliberately out of order: CLAP calls an unsorted list undefined and
        // real plugins crash on one, so the boundary sorts and no caller has to
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

    let seen = param_log();
    assert_eq!(
        seen.len(),
        9,
        "three events a block, three blocks: {seen:?}"
    );
    assert_eq!(
        seen,
        vec![
            "0:0:7:0.010000",
            "0:128:7:0.020000",
            "0:256:7:0.030000",
            "1:0:7:0.110000",
            "1:128:7:0.120000",
            "1:256:7:0.130000",
            "2:0:7:0.210000",
            "2:128:7:0.220000",
            "2:256:7:0.230000",
        ]
    );
}

#[test]
fn a_block_sorts_its_events_whatever_order_they_arrive_in() {
    let mut block = Block::new();
    block.set_events(&[
        ParamEvent {
            time: 400,
            id: 2,
            value: 1.0,
        },
        ParamEvent {
            time: 0,
            id: 1,
            value: 2.0,
        },
        ParamEvent {
            time: 0,
            id: 3,
            value: 3.0,
        },
    ]);
    let (_, _, events) = block.parts();
    let times: Vec<u32> = events.iter().map(|event| event.header.time).collect();
    assert_eq!(times, vec![0, 0, 400]);
    // Stable: two events at the same frame keep the order they were baked in.
    let ids: Vec<u32> = events.iter().map(|event| event.param_id).collect();
    assert_eq!(ids, vec![1, 3, 2]);
}

// ---------------------------------------------------------- the interleave --

#[test]
fn a_block_de_interleaves_into_planes() {
    let mut block = Block::new();
    let src: Vec<f32> = (0..INTERLEAVED_LEN).map(|index| index as f32).collect();
    block.load(&src);
    assert_eq!(block.input()[0], 0.0, "left, frame nought");
    assert_eq!(block.input()[1], 2.0, "left, frame one");
    assert_eq!(block.input()[BLOCK_FRAMES], 1.0, "right, frame nought");
    assert_eq!(block.input()[BLOCK_FRAMES + 1], 3.0, "right, frame one");
}

#[test]
fn a_short_last_block_is_silent_where_the_sound_ran_out() {
    let mut block = Block::new();
    block.load(&[1.0, 1.0, 1.0, 1.0]);
    assert_eq!(block.input()[0], 1.0);
    assert_eq!(
        block.input()[2],
        0.0,
        "past the end is silence, not rubbish"
    );
    assert_eq!(block.input()[BLOCK_FRAMES + 2], 0.0);
}

// -------------------------------------------------------------- denormals --

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[test]
fn denormals_flush_to_zero_inside_the_guard_and_are_restored_after() {
    use std::hint::black_box;
    let tiny = black_box(f32::MIN_POSITIVE);
    let half = black_box(0.5f32);

    let before = black_box(tiny) * black_box(half);
    assert_ne!(before, 0.0, "a denormal is a real number by default");

    let inside = {
        let _guard = crate::process::Denormals::on();
        black_box(tiny) * black_box(half)
    };
    assert_eq!(inside, 0.0, "flush to zero, as both standards assume");

    let after = black_box(tiny) * black_box(half);
    assert_eq!(after, before, "the thread is given back as it was found");
}

// --------------------------------------------------------- console windows --

/// A broker is a console program and Lumit is a windowed one, so on Windows a
/// spawn without `CREATE_NO_WINDOW` opens a console window per plugin file
/// during the start-up scan — reported against 0.3.0. Nothing in this process
/// can observe whether a child was given a console, so the guard is that the
/// spawn still asks for none.
#[test]
fn the_broker_is_spawned_without_a_console_window() {
    let source = include_str!("ipc/broker.rs");
    assert!(
        source.contains("no_console(&mut command);"),
        "the broker spawn must ask for no console window"
    );
    assert!(
        source.contains("command.creation_flags(CREATE_NO_WINDOW);"),
        "no_console must be CREATE_NO_WINDOW and nothing else"
    );
}
