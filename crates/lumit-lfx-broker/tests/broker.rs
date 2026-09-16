//! What the broker is for: the plugin dies and the session does not
//! (docs/impl/lfx.md §14 item 5).
//!
//! These are the tests that can only be written against a real second process -
//! a broker started with no credential, one that speaks another protocol, one
//! whose plugin aborts partway through a frame, one that never comes back, and
//! one that will not stop talking. The plugin is told to misbehave through the
//! broker's environment, because a plugin in another process cannot be reached
//! any other way.
//!
//! They live in this crate rather than in `lumit-lfx` for one flat Cargo
//! reason: `CARGO_BIN_EXE_lumit-lfx-broker` exists only inside the package that
//! owns the binary, and Cargo does not build a dependency's binaries.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use half::f16;
use lumit_budget::{Ledger, Tier};
use lumit_lfx::describe::Declared;
use lumit_lfx::ipc::proto::{DescribedPlugin, HostMessage, ParamValue, PixelDepth, RectI};
use lumit_lfx::ipc::ring::{RingPlan, RING_MIN_SLOTS};
use lumit_lfx::manifest::{CONTENTS_DIR, FAMILY_NAMES, MANIFEST_FILE};
use lumit_lfx::{
    nothing_disabled, Broker, BrokerConfig, BrokerError, LfxRejection, Picture, ProcessJob,
    MAX_LIVE_INSTANCES,
};
use lumit_lfx_testplug::{Personality, CRASH_ON_FRAME_ENV, HANG_ENV, NOTE_SPAM_ENV, PERSONALITIES};

/// The frame size every test works at. Small on purpose: the ring is sized from
/// it, and a test that allocated a 4K ring per case would be measuring the
/// allocator.
const FRAME: (u32, u32) = (8, 8);

/// The broker's own `MISANSWER_ENV`, spelled here because a binary's private
/// constants do not reach its integration tests - the same reason
/// `LUMIT_LFX_BROKER_PROTOCOL` is spelled out below.
const MISANSWER_ENV: &str = "LUMIT_LFX_BROKER_MISANSWER";

// ------------------------------------------------------------ the scaffold --

/// The test bundle's payload name on this platform.
fn cdylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_lfx_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_lfx_testplug.dylib"
    } else {
        "liblumit_lfx_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
fn built_cdylib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..3 {
        for candidate in [
            dir.join(cdylib_name()),
            dir.join("deps").join(cdylib_name()),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = dir.parent()?;
    }
    None
}

/// The architecture directory this machine's payload goes in.
///
/// One string, because a test builds for the machine it runs on. The **ordered
/// per-platform list** a scan tries - `win-x86_64` then `win-arm64`, and so on -
/// is discovery's (§5.1). What a broker needs is a path,
/// and it is given one.
fn arch_dir() -> &'static str {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "win-arm64"
        } else {
            "win-x86_64"
        }
    } else if cfg!(target_os = "macos") {
        "macos-universal"
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64"
    } else {
        "linux-x86_64"
    }
}

/// The id a personality declares, as text.
fn id_of(personality: Personality) -> String {
    personality.id().to_string_lossy().into_owned()
}

/// The manifest a well-formed bundle of the test plugin carries: every
/// personality, spelled the way its own descriptor answers.
///
/// Written from the fixture's own table rather than by hand, because the
/// re-check this file is largely about compares the two records field for
/// field - a hand-written listing would test the typist.
fn a_manifest(edit: impl Fn(Personality, &mut String)) -> String {
    let mut text = format!("abi_version = {}\n", lumit_lfx_abi::LFX_ABI_VERSION);
    for personality in PERSONALITIES {
        let (major, minor, patch) = personality.version();
        let families: Vec<String> = personality
            .categories()
            .iter()
            .map(|declared| {
                FAMILY_NAMES
                    .iter()
                    .find(|(_, number)| number == declared)
                    .map_or_else(|| "utility".to_owned(), |(name, _)| (*name).to_owned())
            })
            .collect();
        let extensions: Vec<String> = personality
            .required_extensions()
            .iter()
            .map(|id| {
                String::from_utf8_lossy(id)
                    .trim_end_matches('\0')
                    .to_owned()
            })
            .collect();
        let mut entry = format!(
            "\n[[plugin]]\nid = {:?}\nname = {:?}\nvendor = {:?}\nversion = \"{major}.{minor}.{patch}\"\ncategories = [{}]\nrequired_extensions = [{}]\n",
            id_of(personality),
            personality.name().to_string_lossy(),
            lumit_lfx_testplug::VENDOR.to_string_lossy(),
            families
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            extensions
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        edit(personality, &mut entry);
        text.push_str(&entry);
    }
    text
}

/// Lay the test plugin out as a real bundle, and answer with the bundle
/// directory and the payload inside it.
///
/// **`None` means one thing only**: the fixture cdylib was not built, which is
/// the one reason a case in this file is allowed to skip. Everything else here
/// is this test's own doing and fails rather than skips.
fn a_bundle_in(root: &Path, manifest: &str) -> Option<(PathBuf, PathBuf)> {
    let source = built_cdylib()?;
    let bundle = root.join("Test.lfx.bundle");
    let contents = bundle.join(CONTENTS_DIR);
    std::fs::create_dir_all(contents.join(arch_dir())).expect("the bundle directory");
    std::fs::write(contents.join(MANIFEST_FILE), manifest).expect("the manifest");
    let payload = contents.join(arch_dir()).join("Test.lfx");
    std::fs::copy(&source, &payload).expect("the payload");
    Some((bundle, payload))
}

/// Say why a test did nothing, by name, so a skip is never silent.
fn skipped(test: &str) {
    eprintln!(
        "{test}: skipped - {} was not found in the target directory. \
         Build it first: cargo build -p lumit-lfx-testplug",
        cdylib_name()
    );
}

/// A broker over the test bundle, with short deadlines and whatever environment
/// the case wants the plugin to misbehave under.
///
/// **A `None` is a missing fixture and nothing else.** A broker that would not
/// start - a broken handshake ordering, a credential that never arrives, a ring
/// that cannot be made, a `broker_exe` pointing nowhere - is a failure, and the
/// spawn below says so. Folding the two together is how twelve of the tests in
/// this file would pass green while the second process never ran at all.
fn a_broker_over(
    root: &Path,
    manifest: &str,
    env: &[(&str, &str)],
    disabled: &[Personality],
) -> Option<Broker> {
    a_broker_charged_to(root, manifest, env, disabled, &Ledger::new())
}

/// The same broker, charged to a ledger the case chose rather than to the
/// machine's own.
///
/// `Ledger::new()` reads the memory this machine has and affords anything a
/// frame this small asks for, which is what a test that is not about the
/// governor wants. A ledger with nothing in it is how §3.4's floor - three
/// slots, taken over the governor's head and standing on its denial count - is
/// reached on purpose.
fn a_broker_charged_to(
    root: &Path,
    manifest: &str,
    env: &[(&str, &str)],
    disabled: &[Personality],
    ledger: &Arc<Ledger>,
) -> Option<Broker> {
    let (bundle, payload) = a_bundle_in(root, manifest)?;
    let mut config = BrokerConfig::new(
        bundle,
        payload,
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F32),
    );
    // The shipped deadlines are ten seconds and two (docs/12 §2.3). A test that
    // waited them out three times over would take a minute; these are the same
    // numbers a quirks-table entry writes, which is the point - the override is
    // the mechanism, not a test hook.
    config.quirks.process_timeout = Duration::from_millis(400);
    config.quirks.control_timeout = Duration::from_secs(5);
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    config.env = env
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    let list = nothing_disabled();
    if let Ok(mut held) = list.lock() {
        held.extend(disabled.iter().copied().map(id_of));
    }
    config.disabled = list;

    Some(Broker::spawn(config, ledger).expect("a broker"))
}

/// The common case: every personality listed honestly and nothing switched off.
fn a_broker(root: &Path, env: &[(&str, &str)]) -> Option<Broker> {
    a_broker_over(root, &a_manifest(|_, _| {}), env, &[])
}

/// The same bundle, hosted on a ledger with no room at all, which is the one
/// arrangement §3.4's floor and the ceilings above it can be seen from.
fn a_broker_on_a_full_machine(root: &Path, env: &[(&str, &str)]) -> Option<(Broker, Arc<Ledger>)> {
    let ledger = Ledger::with_budgets(1 << 30, 0);
    let broker = a_broker_charged_to(root, &a_manifest(|_, _| {}), env, &[], &ledger)?;
    Some((broker, ledger))
}

/// One value per declaration that carries one, each at the default the plugin
/// itself declared - read off the describe rather than written out here, so a
/// fixture that grows a control does not silently go untested.
fn values_for(plugin: &DescribedPlugin) -> Vec<ParamValue> {
    plugin
        .params
        .iter()
        .filter_map(|declaration| match &declaration.kind {
            Declared::Float { default, .. } => Some(ParamValue::Float(*default)),
            Declared::Slider { default, .. } => Some(ParamValue::Slider(*default)),
            Declared::Int { default, .. } => Some(ParamValue::Int(*default)),
            Declared::Angle { default, .. } => Some(ParamValue::Angle(*default)),
            Declared::Bool { default } => Some(ParamValue::Bool(*default)),
            Declared::Choice { default, .. } => Some(ParamValue::Choice(*default)),
            Declared::Colour { default, .. } => Some(ParamValue::Colour([
                default[0] as f32,
                default[1] as f32,
                default[2] as f32,
                default[3] as f32,
            ])),
            Declared::Seed => Some(ParamValue::Seed(7)),
            Declared::Point2 { default, .. } => {
                Some(ParamValue::Point2([default.0 as f32, default.1 as f32]))
            }
            Declared::Point3 { default, .. } => Some(ParamValue::Point3([
                default.0 as f32,
                default.1 as f32,
                default.2 as f32,
            ])),
            Declared::Curve { default } => Some(ParamValue::Curve(default.clone())),
            Declared::File { .. } => Some(ParamValue::File(None)),
            // An Action carries no value and takes no element of the array.
            Declared::Action => None,
        })
        .collect()
}

/// The described record for one personality, or a skip.
fn described(broker: &Broker, personality: Personality) -> Option<DescribedPlugin> {
    broker
        .described()
        .iter()
        .find(|plugin| plugin.identity.id == id_of(personality))
        .cloned()
}

/// A picture every sample of which is the same value.
fn a_flat_frame(value: f32) -> Picture {
    Picture::F32(vec![value; (FRAME.0 * FRAME.1 * 4) as usize])
}

/// The same picture in halves, which is half as many bytes to a slot.
///
/// The one case that wants it is the prefetch ceiling: it is counted in slots
/// and not in bytes, and a claim about what halving a slot does not lift is
/// measured by nothing unless both depths are driven.
fn a_flat_half_frame(value: f32) -> Picture {
    Picture::F16(vec![f16::from_f32(value); (FRAME.0 * FRAME.1 * 4) as usize])
}

/// A picture twice the frame wide and twice it tall, which is a frame no slot
/// of the ring above holds and therefore a regrow.
fn a_frame_too_big_for_a_slot(value: f32) -> Picture {
    Picture::F32(vec![value; (FRAME.0 * 2 * FRAME.1 * 2 * 4) as usize])
}

/// The whole frame, as the job the render path hands over.
fn a_job<'a>(time: f64, input: &'a Picture) -> ProcessJob<'a> {
    ProcessJob {
        time,
        bounds: RectI::of(FRAME.0, FRAME.1),
        roi: RectI::of(FRAME.0, FRAME.1),
        input,
        neighbours: &[],
    }
}

/// The same job, asking for a region strictly inside the frame.
///
/// The definition is the buffer exactly, as `lfx_frame` says; the region asked
/// for is a rectangle inside it, which is what the header means by "the output
/// region asked for - full-frame is the degenerate case, not the assumption".
fn a_job_over<'a>(time: f64, input: &'a Picture, roi: RectI) -> ProcessJob<'a> {
    ProcessJob {
        time,
        bounds: RectI::of(FRAME.0, FRAME.1),
        roi,
        input,
        neighbours: &[],
    }
}

/// The first sample that came back.
fn first(picture: &Picture) -> f32 {
    match picture {
        Picture::F32(whole) => whole.first().copied().unwrap_or(f32::NAN),
        Picture::F16(halves) => halves.first().copied().unwrap_or(f16::NAN).to_f32(),
    }
}

/// Describe the bundle and make one instance of a personality, with the values
/// its own describe declared.
fn an_instance(broker: &mut Broker, personality: Personality) -> Option<u32> {
    let plugin = described(broker, personality)?;
    let values = values_for(&plugin);
    broker.create_instance(&plugin.identity.id, values).ok()
}

// --------------------------------------------------------------- the tests --

/// The whole point of the ordering: a listing is read, and the module is not
/// opened to answer it.
///
/// The payload here is **not a library at all** - it is a line of text - so a
/// broker that opened the module to answer a manifest could not answer one.
/// The manifest comes back whole; the describe that follows is the refusal,
/// which is the same fact stated from the other side.
#[test]
fn a_manifest_is_read_without_the_module_being_opened() {
    let test = "a_manifest_is_read_without_the_module_being_opened";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    if built_cdylib().is_none() {
        skipped(test);
        return;
    }
    let bundle = root.path().join("Text.lfx.bundle");
    let contents = bundle.join(CONTENTS_DIR);
    std::fs::create_dir_all(contents.join(arch_dir())).expect("the bundle");
    std::fs::write(contents.join(MANIFEST_FILE), a_manifest(|_, _| {})).expect("the manifest");
    let payload = contents.join(arch_dir()).join("Text.lfx");
    std::fs::write(&payload, "this is not a library").expect("the payload");

    let mut config = BrokerConfig::new(
        bundle,
        payload,
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F32),
    );
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    let mut broker = Broker::spawn(config, &Ledger::new()).expect("a broker");

    let listed: Vec<String> = broker
        .manifest()
        .expect("the listing")
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    assert_eq!(
        listed.len(),
        PERSONALITIES.len(),
        "every plugin the listing declares is named without the module being opened"
    );
    assert!(listed.contains(&id_of(Personality::Full)));

    // And the module really could not have been opened, which is what makes the
    // answer above worth having.
    assert!(
        broker.describe().is_err(),
        "a payload that is not a library must refuse once it is actually opened"
    );
}

/// The manifest is the cheap listing and the code is the authority
/// (docs/impl/lfx.md §3.3, §4.3). A listing that says something the module does
/// not refuses that plugin by name and leaves the rest of the bundle alone.
#[test]
fn a_listing_that_disagrees_with_the_code_refuses_that_plugin_by_name() {
    let test = "a_listing_that_disagrees_with_the_code_refuses_that_plugin_by_name";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let manifest = a_manifest(|personality, entry| {
        if personality == Personality::Passthrough {
            *entry = entry.replace(
                &format!(
                    "vendor = {:?}",
                    lumit_lfx_testplug::VENDOR.to_string_lossy()
                ),
                "vendor = \"Somebody else\"",
            );
        }
    });
    let Some(mut broker) = a_broker_over(root.path(), &manifest, &[], &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");

    assert!(
        described(&broker, Personality::Passthrough).is_none(),
        "a plugin whose listing lies is not catalogued"
    );
    let (id, why) = broker
        .refused()
        .iter()
        .find(|(id, _)| *id == id_of(Personality::Passthrough))
        .expect("the refusal");
    assert_eq!(*id, id_of(Personality::Passthrough));
    match why {
        LfxRejection::ManifestMismatch { field, code, .. } => {
            assert_eq!(*field, "vendor");
            assert_eq!(code, &lumit_lfx_testplug::VENDOR.to_string_lossy());
        }
        other => panic!("expected a manifest mismatch, got {other}"),
    }
    assert!(
        described(&broker, Personality::Full).is_some(),
        "and the rest of the bundle is untouched"
    );
}

/// The field §4.3 exists for. Negotiation runs from the manifest, because
/// keeping the module shut is what §3.3 buys, so the required-extension list is
/// the one field that decides whether a plugin is instantiated at all and the
/// one the descriptor had to grow a counterpart for.
///
/// The disagreement is put on the **manifest's** side here rather than the
/// code's, and that is the fixture's limit rather than the check's: a plugin
/// requiring an extension this version does not offer is refused *before*
/// `create` and so never reaches a describe to be compared, and version 1
/// offers no extension at all. The unit half -
/// `a_required_extension_the_manifest_did_not_declare_is_a_mismatch` in
/// `lumit-lfx`'s own suite - asks the same question the other way round.
#[test]
fn a_required_extension_list_that_is_not_the_manifests_is_a_manifest_mismatch() {
    let test = "a_required_extension_list_that_is_not_the_manifests_is_a_manifest_mismatch";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let manifest = a_manifest(|personality, entry| {
        if personality == Personality::Passthrough {
            *entry = entry.replace(
                "required_extensions = []",
                "required_extensions = [\"lfx.temporal\"]",
            );
        }
    });
    let Some(mut broker) = a_broker_over(root.path(), &manifest, &[], &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");

    let (_, why) = broker
        .refused()
        .iter()
        .find(|(id, _)| *id == id_of(Personality::Passthrough))
        .expect("the refusal");
    match why {
        LfxRejection::ManifestMismatch {
            field, manifest, ..
        } => {
            assert_eq!(*field, "required extensions");
            assert!(manifest.contains("lfx.temporal"), "{manifest}");
        }
        other => panic!("expected a manifest mismatch, got {other}"),
    }
}

/// The disable list travels **with** the describe, so a switched-off plugin's
/// `init` never runs at all - the first of the three places a disable reaches
/// (§5.4). The OFX host does not do this today; this one does.
#[test]
fn a_switched_off_plugin_is_never_described_in_the_broker() {
    let test = "a_switched_off_plugin_is_never_described_in_the_broker";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker_over(
        root.path(),
        &a_manifest(|_, _| {}),
        &[],
        &[Personality::Passthrough],
    ) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");

    assert!(
        broker.is_switched_off(&id_of(Personality::Passthrough)),
        "the list the host holds is the one the message carried"
    );
    assert!(
        described(&broker, Personality::Passthrough).is_none(),
        "a switched-off plugin is absent from the description"
    );
    assert!(
        broker
            .refused()
            .iter()
            .all(|(id, _)| *id != id_of(Personality::Passthrough)),
        "and it is not a refusal either: it was never asked"
    );
    assert!(described(&broker, Personality::Full).is_some());
}

/// Every host message the protocol says is answered **is** answered, and
/// answered once: each reply below is the answer to its own question, and a
/// broker that answered any of them twice would leave the spare reply in the
/// queue for the next one to collect, which the sequence would catch.
///
/// The two exempt messages are `Frames` and `Shutdown`, and the host's own
/// supervisor refuses to wait on either - `a_message_with_no_reply_is_never_waited_on`
/// in `lumit-lfx` is that half.
#[test]
fn every_host_message_but_frames_and_shutdown_is_answered_exactly_once() {
    let test = "every_host_message_but_frames_and_shutdown_is_answered_exactly_once";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };

    // Open was answered during the spawn, or the ring would still be reachable
    // by name; the test below asks that directly.
    assert!(!broker.manifest().expect("Manifest").is_empty());
    assert!(!broker.describe().expect("Describe").is_empty());

    let plugin = described(&broker, Personality::Passthrough).expect("the passthrough plugin");
    let values = values_for(&plugin);
    let instance = broker
        .create_instance(&plugin.identity.id, values.clone())
        .expect("CreateInstance");
    broker.set_values(instance, values).expect("Values");
    broker.press(instance, "nothing").expect("Action");

    let input = a_flat_frame(0.5);
    let rendered = broker
        .process(instance, &a_job(0.0, &input))
        .expect("Process");
    assert!((first(&rendered.pixels) - 0.5).abs() < 1e-3);

    broker.destroy(instance);

    // Nothing was left behind: one more full round trip works, which it would
    // not if a spare reply were sitting in the queue.
    let again = an_instance(&mut broker, Personality::Passthrough).expect("a second instance");
    let rendered = broker.process(again, &a_job(1.0, &input)).expect("Process");
    assert!((first(&rendered.pixels) - 0.5).abs() < 1e-3);
    assert_eq!(broker.strikes(), 0, "nothing counted as a failure");
}

/// A value the host wrote is the value the plugin read, across a process
/// boundary and a dense array it walked by the host's own stride.
#[test]
fn a_value_the_host_wrote_reaches_the_plugin_through_the_broker() {
    let test = "a_value_the_host_wrote_reaches_the_plugin_through_the_broker";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let plugin = described(&broker, Personality::Full).expect("the full plugin");
    let mut values = values_for(&plugin);
    // The first declaration that carries a value is the Float this effect
    // multiplies its picture by.
    values[0] = ParamValue::Float(0.25);
    let instance = broker
        .create_instance(&plugin.identity.id, values)
        .expect("an instance");

    let input = a_flat_frame(1.0);
    let rendered = broker
        .process(instance, &a_job(0.0, &input))
        .expect("a frame");
    assert!(
        (first(&rendered.pixels) - 0.25).abs() < 1e-3,
        "the gain the host wrote is the gain the plugin applied: {}",
        first(&rendered.pixels)
    );
}

/// Both depths, end to end and between two processes: an fp16 frame crosses the
/// ring and comes back unchanged, an fp32 frame does, and neither is converted
/// to accommodate the other (docs/12 §3.3, §14 item 6).
#[test]
fn both_depths_cross_the_ring_between_two_processes_unchanged() {
    let test = "both_depths_cross_the_ring_between_two_processes_unchanged";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Passthrough).expect("an instance");

    let whole = a_flat_frame(0.75);
    let back = broker.process(instance, &a_job(0.0, &whole)).expect("fp32");
    assert!(
        matches!(back.pixels, Picture::F32(_)),
        "fp32 came back fp32"
    );
    assert!((first(&back.pixels) - 0.75).abs() < 1e-3);

    let halves = Picture::F16(vec![f16::from_f32(0.75); (FRAME.0 * FRAME.1 * 4) as usize]);
    let back = broker
        .process(instance, &a_job(1.0, &halves))
        .expect("fp16");
    match &back.pixels {
        Picture::F16(got) => assert!(
            got.iter()
                .all(|half| half.to_bits() == f16::from_f32(0.75).to_bits()),
            "the halves came back as the bits that went in"
        ),
        Picture::F32(_) => panic!("an fp16 frame came back as floats"),
    }
    assert_eq!(broker.strikes(), 0);
}

/// **Identity means identity, through the broker as well as in process.**
///
/// A plugin that answers `LFX_STATUS_OK` and writes nothing at all has rendered
/// identity - that is what the ABI means by it, and it is not the input copied
/// back, which would put the picture through the depth boundary and change it
/// very slightly. The in-process half pins that against a caller-owned buffer;
/// this is the half that crosses two processes and a ring, where "as it found
/// it" is whatever the broker put in the output buffer before the call. A
/// zero-filled one would make this plugin's every frame a black picture that
/// comes back `Ok` - no strike, no sentence, no badge.
///
/// The halves are compared by bits rather than by value, for the reason
/// `both_depths_cross_the_ring_between_two_processes_unchanged` gives.
#[test]
fn a_plugin_that_writes_nothing_renders_identity_through_the_broker() {
    let test = "a_plugin_that_writes_nothing_renders_identity_through_the_broker";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Identity).expect("an instance");

    // Something with a bit pattern worth telling from nought: an fp16 frame
    // whose samples are not all alike.
    let halves: Vec<f16> = (0..(FRAME.0 * FRAME.1 * 4))
        .map(|index| f16::from_f32(0.125 + index as f32 / 1024.0))
        .collect();
    let input = Picture::F16(halves.clone());
    let rendered = broker
        .process(instance, &a_job(0.0, &input))
        .expect("identity is a success, not a failure");
    match &rendered.pixels {
        Picture::F16(got) => {
            let before: Vec<u16> = halves.iter().map(|half| half.to_bits()).collect();
            let after: Vec<u16> = got.iter().map(|half| half.to_bits()).collect();
            assert_eq!(after, before, "byte for byte");
        }
        Picture::F32(_) => panic!("an fp16 frame came back as floats"),
    }
    assert_eq!(broker.strikes(), 0, "nothing counted as a failure");
}

/// A region the plugin never wrote comes back as the input rather than as
/// nought.
///
/// `ProcessJob::roi` is the output region asked for and the definition is the
/// whole buffer, so a plugin that honours a partial region leaves everything
/// outside it untouched - and what "untouched" comes back as is whatever the
/// broker seeded the output with. The fixture has no ROI-honouring personality
/// (`shade` writes the whole buffer whatever it is asked for), so the case is
/// driven through the one that writes nothing at all, where the untouched
/// margin is the whole frame; `lfx-validator`'s ROI honesty check is the fuller
/// half and belongs to the validator.
#[test]
fn a_region_the_plugin_never_wrote_comes_back_as_the_input() {
    let test = "a_region_the_plugin_never_wrote_comes_back_as_the_input";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Identity).expect("an instance");

    let input = a_flat_frame(0.5);
    let middle = RectI {
        x0: 2,
        y0: 2,
        x1: 6,
        y1: 6,
    };
    let rendered = broker
        .process(instance, &a_job_over(0.0, &input, middle))
        .expect("a frame");
    match (&rendered.pixels, &input) {
        (Picture::F32(got), Picture::F32(sent)) => {
            assert_eq!(got.len(), sent.len(), "the whole buffer came back");
            assert!(
                got.iter()
                    .all(|sample| (*sample - 0.5).abs() < f32::EPSILON),
                "the margin outside the region asked for came back as nought"
            );
        }
        _ => panic!("an fp32 frame came back at another depth"),
    }
}

/// A plugin that aborts partway through a frame costs one frame. The broker is
/// replaced, every instance is made again from the records the host holds, and
/// the next frame renders.
#[test]
fn a_crash_on_a_frame_restarts_the_broker_and_the_session_carries_on() {
    let test = "a_crash_on_a_frame_restarts_the_broker_and_the_session_carries_on";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[(CRASH_ON_FRAME_ENV, "5")]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Crash).expect("an instance");

    let input = a_flat_frame(0.5);
    let before = broker
        .process(instance, &a_job(1.0, &input))
        .expect("frame 1");
    assert!((first(&before.pixels) - 0.5).abs() < 1e-3);

    assert!(
        broker.process(instance, &a_job(5.0, &input)).is_err(),
        "the frame the plugin died on is lost"
    );
    assert_eq!(broker.restarts(), 1, "and the broker was started again");
    assert!(!broker.is_disabled(), "one death is not three");

    // The replay: the same handle names the same plugin in the new process,
    // with the values the host held.
    let after = broker
        .process(instance, &a_job(6.0, &input))
        .expect("frame 6");
    assert!(
        (first(&after.pixels) - 0.5).abs() < 1e-3,
        "the session carried on"
    );
    assert_eq!(
        broker.strikes(),
        0,
        "a success puts the count back to nought"
    );
}

/// A plugin that never comes back trips the deadline, and three consecutive
/// strikes put it away for the session. Every frame after that comes back with
/// a sentence, so the layer renders identity and wears a badge.
#[test]
fn a_hang_trips_the_deadline_and_the_third_strike_disables_the_plugin() {
    let test = "a_hang_trips_the_deadline_and_the_third_strike_disables_the_plugin";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[(HANG_ENV, "1")]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Hang).expect("an instance");

    let input = a_flat_frame(0.5);
    for attempt in 1..=3 {
        let outcome = broker.process(instance, &a_job(f64::from(attempt), &input));
        assert!(outcome.is_err(), "attempt {attempt} should have timed out");
    }
    assert!(
        broker.is_disabled(),
        "three consecutive failures put the plugin away for the session"
    );
    assert_eq!(
        broker.strikes(),
        lumit_lfx::ipc::broker::STRIKES_BEFORE_DISABLED
    );

    let after = broker.process(instance, &a_job(9.0, &input));
    assert!(after.is_err(), "and it stays away");
}

/// A broker that answers a question nobody asked has fallen out of step on the
/// pipe, and that is a strike rather than a success.
///
/// It is the one failure a supervisor looking only for a `Failed` counts as
/// going well: the reply arrived, the deadline held, the process is alive - so
/// the strike count goes back to nought, and a broker answering nonsense for
/// ever never reaches three *consecutive* strikes, is never replaced and is
/// never put away. Held to [`HostMessage::answers`] instead, each answer out of
/// turn strikes and replaces the broker, and the third stops trying
/// (docs/impl/lfx.md §3.5).
#[test]
fn an_answer_to_a_question_nobody_asked_is_a_strike_rather_than_a_reset() {
    let test = "an_answer_to_a_question_nobody_asked_is_a_strike_rather_than_a_reset";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[(MISANSWER_ENV, "reply")]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Passthrough).expect("an instance");
    let input = a_flat_frame(0.5);

    for attempt in 1..=2 {
        match broker.process(instance, &a_job(f64::from(attempt), &input)) {
            Err(BrokerError::Unexpected(answered)) => assert_eq!(
                answered, "Created",
                "the answer is named rather than guessed at"
            ),
            other => panic!("attempt {attempt}: {:?}", other.err()),
        }
        assert_eq!(
            broker.strikes(),
            attempt,
            "a reply out of turn did not put the count back to nought"
        );
    }
    assert_eq!(
        broker.restarts(),
        2,
        "and each of the first two replaced the broker exactly once, which is out of step rather than merely unwilling"
    );

    let third = broker.process(instance, &a_job(3.0, &input));
    assert!(
        matches!(third, Err(BrokerError::Disabled)),
        "{third:?}",
        third = third.err()
    );
    assert!(
        broker.is_disabled(),
        "three consecutive answers out of turn is three strikes"
    );
}

/// A plugin that will not stop talking fills nothing: the host keeps the last
/// few lines and the frame still renders.
#[test]
fn a_plugin_that_will_not_stop_talking_does_not_fill_the_host() {
    let test = "a_plugin_that_will_not_stop_talking_does_not_fill_the_host";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[(NOTE_SPAM_ENV, "500")]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::NoteSpam).expect("an instance");

    let input = a_flat_frame(0.5);
    let rendered = broker
        .process(instance, &a_job(0.0, &input))
        .expect("a frame");
    assert!((first(&rendered.pixels) - 0.5).abs() < 1e-3);
    assert!(
        broker.notes().len() <= lumit_lfx::MAX_NOTES,
        "the host keeps the last few lines, not all of them"
    );
    assert!(
        !broker.notes().is_empty(),
        "and it does keep some: a plugin that says something must be heard"
    );
}

/// A handle nobody minted is **answered**, never followed - and the rule has
/// two halves, in two places.
///
/// The **host's** half is that a handle this host does not hold is answered
/// here, without the pipe being touched: the host owns the record for every
/// live instance, so a message about one it has never heard of is a message
/// with nothing to say. It costs no strike, which is the point - a press racing
/// a layer deletion is three `Failed`s and a disabled bundle otherwise, for a
/// button the user was entitled to press.
///
/// The **broker's** half is that a forged handle that does reach it is a
/// `Failed` at every entry point rather than "unsupported", which would tell a
/// plugin the feature is missing when the truth is its handle is rubbish. Once
/// the host's half is kept, nothing in the shipping path can reach it - so the
/// seam that can is named for what it is and used here.
#[test]
fn a_handle_the_host_never_minted_is_answered_rather_than_followed() {
    let test = "a_handle_the_host_never_minted_is_answered_rather_than_followed";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let real = an_instance(&mut broker, Personality::Passthrough).expect("an instance");
    let input = a_flat_frame(0.5);

    // A plain counter, which is what a host that did not mint handles would
    // send: the right shape and none of the magic. Every guarded entry point
    // answers it here.
    for answered in [
        broker.press(7, "nothing"),
        broker.set_values(7, Vec::new()),
        broker.process(7, &a_job(0.0, &input)).map(|_| ()),
    ] {
        match answered {
            Err(BrokerError::NoSuchInstance { instance }) => assert_eq!(instance, 7),
            other => panic!("a forged handle must be answered by the host: {other:?}"),
        }
    }
    assert_eq!(
        broker.strikes(),
        0,
        "a handle the host answered never reached the plugin, so nothing failed"
    );
    assert_eq!(broker.restarts(), 0);

    // A handle the host held and does not any more is the same answer: the
    // record goes with the instance.
    broker.destroy(real);
    match broker.press(real, "nothing") {
        Err(BrokerError::NoSuchInstance { .. }) => {}
        other => panic!("a destroyed instance must be answered: {other:?}"),
    }
    assert_eq!(broker.strikes(), 0);

    // And the broker's own half, through the one route that reaches it: every
    // entry point that takes a handle answers `Failed`, never "unsupported".
    // A successful frame between them puts the strike count back to nought,
    // which is what keeps three refusals in a row from being three refusals.
    let again = an_instance(&mut broker, Personality::Passthrough).expect("a second instance");
    for forged in [
        HostMessage::Action {
            instance: 7,
            param: "nothing".to_owned(),
        },
        HostMessage::Values {
            instance: 7,
            values: Vec::new(),
        },
        HostMessage::Destroy { instance: 7 },
    ] {
        let name = forged.name();
        match broker.ask_with_a_forged_handle_for_test(&forged) {
            Err(BrokerError::Refused(why)) => {
                assert!(why.contains("no such instance"), "{name}: {why}");
            }
            other => panic!("{name} must answer a forged handle: {other:?}"),
        }
        assert_eq!(
            broker.strikes(),
            1,
            "{name}: a refusal is a strike and not a restart"
        );
        assert_eq!(broker.restarts(), 0);

        // And the process the forged handle never reached is still there.
        let rendered = broker.process(again, &a_job(0.0, &input)).expect("a frame");
        assert!((first(&rendered.pixels) - 0.5).abs() < 1e-3);
        assert_eq!(broker.strikes(), 0);
    }
}

/// The host reads the slot **it** chose, and the frame that comes back is the
/// size it asked for, or neither is believed - and neither is a success.
///
/// A hash says only that the bytes are the ones the writer meant, and the
/// writer is a process holding a stranger's compiled code. So a broker that
/// answered with the input slot would have the input served as the render -
/// no strike, no sentence, no badge - and one that wrote a one-pixel frame
/// would hand the caller four samples where a whole picture was asked for. The
/// ring applies this reasoning to a header against its own payload one level
/// down; this is the same question one level up, where the ring cannot see it.
///
/// **Refusing the frame is half of it.** An answer of the right kind carrying
/// the wrong content is a process out of step, so it strikes and replaces the
/// broker, exactly as an answer out of turn does. Counting it as a success -
/// which is what putting the count back to nought as soon as the *kind* matched
/// would do - leaves a broker that misanswers every frame inside its deadline,
/// alive, never replaced and never put away, badging the layer for the length
/// of the session. So the count is driven here rather than asserted once
/// (docs/impl/lfx.md §3.5).
#[test]
fn a_frame_that_is_not_the_one_asked_for_is_refused_rather_than_served() {
    let test = "a_frame_that_is_not_the_one_asked_for_is_refused_rather_than_served";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let input = a_flat_frame(0.5);

    for misanswer in ["slot", "size"] {
        let Some(mut broker) = a_broker(root.path(), &[(MISANSWER_ENV, misanswer)]) else {
            skipped(test);
            return;
        };
        broker.describe().expect("a description");
        let instance = an_instance(&mut broker, Personality::Passthrough).expect("an instance");

        for attempt in 1..=2 {
            match broker.process(instance, &a_job(f64::from(attempt), &input)) {
                Err(BrokerError::WrongSlot { asked, answered }) => {
                    assert_eq!(misanswer, "slot");
                    assert_ne!(asked, answered);
                }
                Err(BrokerError::WrongFrame {
                    wide,
                    tall,
                    wanted_wide,
                    ..
                }) => {
                    assert_eq!(misanswer, "size");
                    assert_eq!((wide, tall), (1, 1));
                    assert_eq!(wanted_wide, FRAME.0);
                }
                other => panic!(
                    "{misanswer}: a frame that is not the one asked for must be refused: {:?}",
                    other.err()
                ),
            }
            assert_eq!(
                broker.strikes(),
                attempt,
                "{misanswer}: the wrong answer did not put the count back to nought"
            );
        }
        assert_eq!(
            broker.restarts(),
            2,
            "{misanswer}: and each of the first two replaced the broker, which is out of step rather than merely unwilling"
        );

        let third = broker.process(instance, &a_job(3.0, &input));
        assert!(
            matches!(third, Err(BrokerError::Disabled)),
            "{misanswer}: {third:?}",
            third = third.err()
        );
        assert!(
            broker.is_disabled(),
            "{misanswer}: three consecutive wrong answers is three strikes"
        );
    }
}

/// A broker that speaks another protocol is refused - and refused **after** the
/// proof, never before, so an impostor is not told which build it faces.
#[test]
fn a_broker_that_speaks_another_protocol_is_refused_after_the_proof() {
    let test = "a_broker_that_speaks_another_protocol_is_refused_after_the_proof";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((bundle, payload)) = a_bundle_in(root.path(), &a_manifest(|_, _| {})) else {
        skipped(test);
        return;
    };
    let mut config = BrokerConfig::new(
        bundle,
        payload,
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F32),
    );
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    config.env = vec![("LUMIT_LFX_BROKER_PROTOCOL".to_owned(), "99".to_owned())];

    match Broker::spawn(config, &Ledger::new()) {
        Err(BrokerError::ProtocolMismatch { theirs, ours }) => {
            assert_eq!(theirs, 99);
            assert_eq!(ours, lumit_lfx::ipc::proto::PROTOCOL_VERSION);
        }
        Err(other) => panic!("expected a version refusal, got {other}"),
        Ok(_) => panic!("a broker of another version must be refused, not believed"),
    }
}

/// A broker with no credential refuses rather than serves.
///
/// This is the case that matters most, because it is the one a mistake would
/// quietly re-introduce: if a missing credential meant "carry on anyway", every
/// protection the handshake buys would be one refactor from being optional.
#[test]
fn a_broker_with_no_credential_does_not_start() {
    let test = "a_broker_with_no_credential_does_not_start";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((_, payload)) = a_bundle_in(root.path(), &a_manifest(|_, _| {})) else {
        skipped(test);
        return;
    };

    // Run the broker by hand, the way an impostor would have to: a pipe name it
    // was given, and nothing on standard input.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_lumit-lfx-broker"))
        .arg(&payload)
        .arg("lumit-lfx-nobody-is-listening.sock")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the broker starts");
    let status = child.wait().expect("the broker exits");
    assert!(
        !status.success(),
        "a broker with no credential must refuse rather than serve"
    );
}

/// The endpoint names are unguessable, and no two are alike.
///
/// A name that was the host's process id and a counter is one any program on
/// the machine - including another broker, running somebody else's plugin code -
/// could work out and connect to first.
///
/// **The endpoint is what the clause is about**, so the endpoint is what is
/// read. The ring's name comes out of the same `next_identifier`, so measuring
/// only the ring would go on passing if `pipe::pipe_name` alone were changed
/// while the endpoint became guessable - and it is the endpoint another program
/// connects to. The ring is asserted beside it, because a shared ring is its own
/// failure.
#[test]
fn two_brokers_do_not_share_a_name_and_neither_name_is_a_process_id() {
    let test = "two_brokers_do_not_share_a_name_and_neither_name_is_a_process_id";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(first_broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    // The fixture is there - the first broker proves it - so a second one that
    // will not start is a failure rather than a reason to say nothing.
    let second_broker = a_broker(root.path(), &[]).expect("a second broker");

    let brokers = [&first_broker, &second_broker];
    for (what, names) in [
        (
            "endpoint",
            brokers
                .iter()
                .map(|broker| broker.endpoint_name_for_test())
                .collect::<Vec<String>>(),
        ),
        (
            "ring",
            brokers
                .iter()
                .map(|broker| broker.ring_path_for_test())
                .collect::<Vec<String>>(),
        ),
    ] {
        assert_ne!(
            names.first(),
            names.get(1),
            "two brokers shared one {what} name"
        );
        for name in &names {
            assert!(!name.is_empty(), "a broker has no {what} name at all");
            let stem: String = name
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(name)
                .chars()
                .filter(|c| c.is_ascii_hexdigit())
                .collect();
            assert!(
                stem.len() >= 32,
                "the {what} name carries no unguessable part: {name}"
            );
            assert!(
                !name.contains(&format!("-{}-", std::process::id())),
                "the {what} name still carries this process's id: {name}"
            );
        }
    }
}

/// On Unix the ring's name comes out of the directory once both processes have
/// it mapped. The ring keeps working - a frame still crosses it - and the file
/// is gone from the temporary directory, so no third program can open it and
/// the kernel reclaims it when the last of the two exits, a crash included.
#[cfg(unix)]
#[test]
fn the_ring_file_is_unlinked_once_both_ends_hold_it() {
    let test = "the_ring_file_is_unlinked_once_both_ends_hold_it";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };

    let path = broker.ring_path_for_test();
    assert!(
        !Path::new(&path).exists(),
        "the ring is still reachable by name at {path}"
    );

    // And it is still a working ring, which is the half that would be easy to
    // break: an unlinked mapping is only correct because the mapping outlives
    // the name.
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Passthrough).expect("an instance");
    let input = a_flat_frame(0.75);
    let rendered = broker
        .process(instance, &a_job(0.0, &input))
        .expect("a frame");
    assert!((first(&rendered.pixels) - 0.75).abs() < 1e-3);
}

/// The ring is sized from the window the bundle's plugins declared, not only
/// from the budget - the answer only LFX can give, because only LFX knows the
/// window before the first frame rather than at one (§3.4). The second process
/// maps whatever the first one made, so a frame crossing it afterwards is what
/// proves the regrow reached both ends.
#[test]
fn a_declared_window_sizes_the_ring_the_second_process_maps() {
    let test = "a_declared_window_sizes_the_ring_the_second_process_maps";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    let before = broker.ring_path_for_test();
    broker.describe().expect("a description");

    let temporal = described(&broker, Personality::Temporal).expect("the temporal plugin");
    assert_eq!(
        (temporal.traits.temporal_lo, temporal.traits.temporal_hi),
        (-1, 1),
        "the fixture declares the window this test is about"
    );
    assert!(
        broker.ring_slots() >= 4,
        "a window of [-1, 1] asks for its own frames plus the one being written"
    );
    // And the count the pool above reads without this broker's lock is the
    // count the broker has, after a regrow as before one: §4.4's ceiling is
    // published rather than copied, because a ring is replaced mid-session and
    // a bigger frame buys fewer slots.
    assert_eq!(
        broker.granted_slots().get(),
        broker.ring_slots(),
        "the published slot count followed the regrow"
    );

    // Whether the ring was replaced or was wide enough already, both processes
    // agree about it: a frame crosses.
    let instance = an_instance(&mut broker, Personality::Temporal).expect("an instance");
    let input = a_flat_frame(0.5);
    let rendered = broker
        .process(instance, &a_job(0.0, &input))
        .expect("a frame");
    assert!((first(&rendered.pixels) - 0.5).abs() < 1e-3);
    assert!(
        !broker.ring_path_for_test().is_empty() && !before.is_empty(),
        "there was a ring before and there is one now"
    );
    assert_eq!(
        broker.granted_slots().get(),
        broker.ring_slots(),
        "and it still follows it after a frame has crossed"
    );
    assert_eq!(
        rendered.frames_needed,
        vec![-1, 1],
        "and the instance asks for the neighbours it declared"
    );
}

/// A ledger with no room buys the floor, and having bought it the host stops
/// asking.
///
/// `Ring::create` halves its way down until the governor says yes and takes
/// `RING_MIN_SLOTS` over its head when it never does, so a ring on a full
/// machine holds fewer slots than the plan that asked for it. A `fit` that
/// measured the wish against the answer would find that ring too small for the
/// very plan it *is* and rebuild it for every frame - a file, a mapping, a
/// reservation the ledger has already refused and a control round trip, per
/// frame, for ever. The ring's own name is the witness: a replacement gets a
/// new one (docs/impl/lfx.md §3.4).
#[test]
fn a_ring_the_ledger_narrowed_is_not_made_again_for_every_frame() {
    let test = "a_ring_the_ledger_narrowed_is_not_made_again_for_every_frame";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((mut broker, ledger)) = a_broker_on_a_full_machine(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    assert_eq!(
        broker.ring_slots(),
        RING_MIN_SLOTS,
        "the floor, taken over the governor's head"
    );
    assert!(
        ledger.denials(Tier::Ram) > 0,
        "and the refusal stands on the ledger's own count rather than nowhere"
    );
    assert!(
        broker
            .report()
            .iter()
            .any(|line| matches!(line, LfxRejection::RingNarrowedByTheLedger { .. })),
        "and the report says which kind of no it was: {:?}",
        broker.report()
    );

    let instance = an_instance(&mut broker, Personality::Passthrough).expect("an instance");
    let input = a_flat_frame(0.5);
    broker
        .process(instance, &a_job(0.0, &input))
        .expect("a frame");
    let ring = broker.ring_path_for_test();
    assert!(!ring.is_empty(), "there is a ring to keep");

    for time in 1..4 {
        let rendered = broker
            .process(instance, &a_job(f64::from(time), &input))
            .expect("a frame");
        assert!((first(&rendered.pixels) - 0.5).abs() < 1e-3);
        assert_eq!(
            broker.ring_path_for_test(),
            ring,
            "the ring was made again for a frame the one it had already held"
        );
    }
    assert_eq!(broker.ring_slots(), RING_MIN_SLOTS);
    assert_eq!(broker.strikes(), 0, "and none of it counted as a failure");
    assert_eq!(broker.restarts(), 0);
}

/// The 4K prefetch ceiling, declared rather than discovered: a shipment wider
/// than the ring has **slots** is refused whole.
///
/// Counted in slots and not in bytes, so a shallower depth does not lift it -
/// and refused by the host before a slot is written, so it costs the plugin
/// nothing: no strike, no restart, and the next frame that fits renders
/// (docs/impl/lfx.md §3.4).
#[test]
fn a_shipment_wider_than_the_ring_is_refused_before_a_slot_is_written() {
    let test = "a_shipment_wider_than_the_ring_is_refused_before_a_slot_is_written";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((mut broker, _ledger)) = a_broker_on_a_full_machine(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    assert_eq!(broker.ring_slots(), RING_MIN_SLOTS);
    let instance = an_instance(&mut broker, Personality::Temporal).expect("an instance");
    let input = a_flat_frame(0.5);

    // The input, the output and two neighbours: four pictures for three slots,
    // at each depth in turn. An fp16 slot is half an fp32 one, so a ceiling
    // counted in bytes would lift here and a ceiling counted in slots does not.
    for deep in [true, false] {
        let shipped = if deep {
            a_flat_frame(0.5)
        } else {
            a_flat_half_frame(0.5)
        };
        let neighbours = if deep {
            [(-1.0, a_flat_frame(0.25)), (1.0, a_flat_frame(0.75))]
        } else {
            [
                (-1.0, a_flat_half_frame(0.25)),
                (1.0, a_flat_half_frame(0.75)),
            ]
        };
        let job = ProcessJob {
            time: 0.0,
            bounds: RectI::of(FRAME.0, FRAME.1),
            roi: RectI::of(FRAME.0, FRAME.1),
            input: &shipped,
            neighbours: &neighbours,
        };
        match broker.process(instance, &job) {
            Err(BrokerError::RingTooSmall { wanted, slots }) => {
                assert_eq!(wanted, 4, "the input, the output and both neighbours");
                assert_eq!(
                    slots, RING_MIN_SLOTS as usize,
                    "a shallower depth buys more bytes to a slot and no more slots"
                );
            }
            other => panic!("a shipment of four into three slots: {:?}", other.err()),
        }
        assert_eq!(
            broker.strikes(),
            0,
            "the host refused it, so nothing reached the plugin to be struck for"
        );
        assert_eq!(broker.restarts(), 0);
    }

    // And the ceiling is this shipment's, not the session's: one that fits
    // still renders.
    let rendered = broker
        .process(instance, &a_job(0.0, &input))
        .expect("a frame that fits");
    assert!((first(&rendered.pixels) - 0.5).abs() < 1e-3);
}

/// A ring that cannot be made a second time reaches the watchdog rather than
/// leaving the broker without one for the rest of the session.
///
/// A regrow drops the ring it is replacing before it asks for the new one, so
/// the one transient that matters - a full disk, no file handles left - leaves
/// this broker holding no ring at all. Answering `NoRing` from then on would
/// badge every frame for ever with nothing counting it: the watchdog's three
/// strikes could not fire, so the bundle would be neither mended nor put away.
/// The directory the ring's file goes in is taken away here, which is the one
/// way to reach that failure from outside the host (docs/impl/lfx.md §3.4).
#[test]
fn a_ring_that_cannot_be_made_again_reaches_the_watchdog() {
    let test = "a_ring_that_cannot_be_made_again_reaches_the_watchdog";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let rings = root.path().join("rings");
    if std::fs::create_dir_all(&rings).is_err() {
        skipped(test);
        return;
    }
    let Some((bundle, payload)) = a_bundle_in(root.path(), &a_manifest(|_, _| {})) else {
        skipped(test);
        return;
    };
    let mut config = BrokerConfig::new(
        bundle,
        payload,
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F32),
    );
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    config.quirks.process_timeout = Duration::from_millis(400);
    config.quirks.control_timeout = Duration::from_secs(5);
    config.ring_dir = Some(rings.clone());
    let mut broker = Broker::spawn(config, &Ledger::new()).expect("a broker");
    broker.describe().expect("a description");
    let instance = an_instance(&mut broker, Personality::Passthrough).expect("an instance");

    // From here no ring can be made at all, and the frame below is bigger than
    // a slot, so every one of these frames asks for one.
    std::fs::remove_dir_all(&rings).expect("the ring's directory goes away");
    let bigger = a_frame_too_big_for_a_slot(0.5);
    let job = ProcessJob {
        time: 0.0,
        bounds: RectI::of(FRAME.0 * 2, FRAME.1 * 2),
        roi: RectI::of(FRAME.0 * 2, FRAME.1 * 2),
        input: &bigger,
        neighbours: &[],
    };

    for attempt in 1..=2 {
        let answer = broker.process(instance, &job);
        assert!(
            answer.is_err(),
            "a frame with no ring to write it into is not a frame"
        );
        assert_eq!(
            broker.strikes(),
            attempt,
            "a ring that could not be made is counted rather than sat in"
        );
    }
    let third = broker.process(instance, &job);
    assert!(
        matches!(third, Err(BrokerError::Disabled)),
        "{third:?}",
        third = third.err()
    );
    assert!(
        broker.is_disabled(),
        "a machine that cannot give this session a ring puts the bundle away rather than badging every frame for ever"
    );
}

/// The answer to the `Open` that hands over the ring is held to what that
/// message admits, the same way every other answer is held to its question.
///
/// This is the one exchange the host runs outside its own supervisor, so it is
/// the one place a reply to another question could be taken for "the ring is
/// mapped": the name would be unlinked on a mapping that never happened, the
/// host would write frames into a ring nobody is reading, and the answer really
/// owed to the `Open` would still be on the pipe for the next question to
/// collect. `HostMessage::answers` is read here rather than copied
/// (docs/impl/lfx.md §3.5).
#[test]
fn an_answer_to_an_open_is_held_to_what_the_open_admits() {
    let test = "an_answer_to_an_open_is_held_to_what_the_open_admits";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((bundle, payload)) = a_bundle_in(root.path(), &a_manifest(|_, _| {})) else {
        skipped(test);
        return;
    };
    let mut config = BrokerConfig::new(
        bundle,
        payload,
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F32),
    );
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    config.env = vec![(MISANSWER_ENV.to_owned(), "open".to_owned())];

    match Broker::spawn(config, &Ledger::new()) {
        Err(BrokerError::Unexpected(answered)) => assert_eq!(
            answered, "Created",
            "the answer is named rather than guessed at"
        ),
        Err(other) => panic!("expected an answer out of turn, got {other}"),
        Ok(_) => panic!("a broker that answered the Open with somebody else's reply must not be taken for one holding the ring"),
    }
}

/// The line that says why a prefetch is refused is filed again by the broker
/// that replaces one, because the replacement ring is just as narrow.
///
/// A restart takes the new broker's own report - it describes again, and the
/// describe carries the report of the bundle's own refusals - so the lines the
/// **host** filed about its ring go with the old one. The new ring is made from
/// the same plan on the same pressed machine, so the refusals carry on; an
/// Addons page that stopped explaining them after one crash would be the page
/// disagreeing with the layer (docs/impl/lfx.md §3.4).
#[test]
fn a_narrowed_rings_line_is_filed_again_by_the_broker_that_replaces_it() {
    let test = "a_narrowed_rings_line_is_filed_again_by_the_broker_that_replaces_it";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((mut broker, _ledger)) =
        a_broker_on_a_full_machine(root.path(), &[(CRASH_ON_FRAME_ENV, "2")])
    else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let narrowed = |broker: &Broker| {
        broker
            .report()
            .iter()
            .filter(|line| matches!(line, LfxRejection::RingNarrowedByTheLedger { .. }))
            .count()
    };
    assert_eq!(narrowed(&broker), 1, "{:?}", broker.report());

    let instance = an_instance(&mut broker, Personality::Crash).expect("an instance");
    let input = a_flat_frame(0.5);
    broker
        .process(instance, &a_job(1.0, &input))
        .expect("frame 1");
    assert!(
        broker.process(instance, &a_job(2.0, &input)).is_err(),
        "the frame the plugin died on is lost"
    );
    assert_eq!(broker.restarts(), 1, "and the broker was started again");

    assert_eq!(
        narrowed(&broker),
        1,
        "the replacement ring is as narrow as the one it replaced, and says so exactly once: {:?}",
        broker.report()
    );
}

/// One bundle holds no more live instances than a replay can carry, and the
/// ceiling is the host's own: it is answered without the pipe being touched, so
/// a caller that kept asking never strikes the plugin for it.
///
/// `MAX_LIVE_INSTANCES` bounds two things rather than one - the memory a
/// runaway caller can ask a broker for, and the length of the replay a restart
/// has to perform (docs/impl/lfx.md §3.5).
#[test]
fn a_bundle_may_not_hold_more_instances_than_a_replay_can_carry() {
    let test = "a_bundle_may_not_hold_more_instances_than_a_replay_can_carry";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let plugin = described(&broker, Personality::Passthrough).expect("the passthrough plugin");
    let values = values_for(&plugin);

    let mut made = Vec::with_capacity(MAX_LIVE_INSTANCES);
    for _ in 0..MAX_LIVE_INSTANCES {
        made.push(
            broker
                .create_instance(&plugin.identity.id, values.clone())
                .expect("an instance"),
        );
    }
    assert_eq!(broker.live_instances(), MAX_LIVE_INSTANCES);

    match broker.create_instance(&plugin.identity.id, values.clone()) {
        Err(BrokerError::TooManyInstances { limit }) => assert_eq!(limit, MAX_LIVE_INSTANCES),
        other => panic!("one past the ceiling: {:?}", other.err()),
    }
    assert_eq!(
        broker.strikes(),
        0,
        "the ceiling is the host's, so nothing was sent to be struck for"
    );

    // And it is a ceiling on the live ones rather than on the session: letting
    // one go makes room for another.
    broker.destroy(made[0]);
    assert_eq!(broker.live_instances(), MAX_LIVE_INSTANCES - 1);
    broker
        .create_instance(&plugin.identity.id, values)
        .expect("room again");
}

/// The listing a bundle carries is read under `lumit-ingress`, in the broker,
/// and a bundle with no listing at all is a calm line rather than a fault.
#[test]
fn a_bundle_with_no_listing_is_refused_with_a_sentence() {
    let test = "a_bundle_with_no_listing_is_refused_with_a_sentence";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((bundle, payload)) = a_bundle_in(root.path(), &a_manifest(|_, _| {})) else {
        skipped(test);
        return;
    };
    std::fs::remove_file(bundle.join(CONTENTS_DIR).join(MANIFEST_FILE)).expect("no listing");

    let mut config = BrokerConfig::new(
        bundle,
        payload,
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F32),
    );
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    let mut broker = Broker::spawn(config, &Ledger::new()).expect("a broker");
    match broker.manifest() {
        Err(BrokerError::Refused(why)) => {
            assert!(why.contains("manifest"), "{why}");
        }
        other => panic!("a bundle with no listing must be refused: {other:?}"),
    }
}

/// Nothing switched off is the ordinary case, and the list the describe carries
/// is empty rather than absent - a distinction that matters because an empty
/// `BTreeSet` is what a fresh session has.
#[test]
fn a_bundle_with_nothing_switched_off_describes_everything_that_can_be() {
    let test = "a_bundle_with_nothing_switched_off_describes_everything_that_can_be";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    let plugins = broker.describe().expect("a description");
    let ids: BTreeSet<String> = plugins
        .iter()
        .map(|plugin| plugin.identity.id.clone())
        .collect();

    for present in [
        Personality::Full,
        Personality::Slim,
        Personality::Temporal,
        Personality::Passthrough,
        Personality::Identity,
    ] {
        assert!(
            ids.contains(&id_of(present)),
            "{} is missing",
            id_of(present)
        );
    }
    // Three personalities cannot be catalogued and each is a different refusal:
    // one declines to describe, one puts two controls on one id, and one
    // requires an extension version 1 does not offer - which is refused
    // **before** it is instantiated (§4.3).
    //
    // **Each is named with its own reason**, which is the whole of what §5.3's
    // `REFUSED` table is made of. A plugin dropped in the second process with
    // no line anywhere would reach the host as an absence and nothing else, and
    // an absence is what a plugin the user switched off looks like too.
    for (absent, expected) in [
        (
            Personality::BrokenDescribe,
            "a plugin that declines to describe itself",
        ),
        (
            Personality::DuplicateIds,
            "two controls that would drive each other",
        ),
        (
            Personality::MissingExtension,
            "an extension this host does not offer",
        ),
    ] {
        assert!(
            !ids.contains(&id_of(absent)),
            "{} should not be catalogued",
            id_of(absent)
        );
        let (_, why) = broker
            .refused()
            .iter()
            .find(|(id, _)| *id == id_of(absent))
            .unwrap_or_else(|| panic!("{} was dropped with no sentence", id_of(absent)));
        match (absent, why) {
            (Personality::BrokenDescribe, LfxRejection::DescribeRefused { id }) => {
                assert_eq!(*id, id_of(absent));
            }
            // The sink's own fault, crossing the pipe as itself: the two rows
            // that would have driven each other are named rather than rendered
            // into a sentence in the second process.
            (Personality::DuplicateIds, LfxRejection::DuplicateParamId { first, second }) => {
                assert_eq!(first, second);
            }
            (Personality::MissingExtension, LfxRejection::RequiresExtension { id, extension }) => {
                assert_eq!(*id, id_of(absent));
                assert_eq!(extension, "lfx.gpu-frames");
            }
            (_, other) => panic!("{expected} came back as {other}"),
        }
    }
    // And none of the three is a manifest disagreement: the listing and the
    // code agree about every one of them, which is what makes the refusal above
    // the describe's own rather than the re-check's.
    assert!(
        broker
            .refused()
            .iter()
            .all(|(_, why)| !matches!(why, LfxRejection::ManifestMismatch { .. })),
        "{:?}",
        broker.refused()
    );
}

// ---------------------------------------------------------------------------
// The catalogue entry, end to end (docs/impl/lfx.md §4.2, §14 item 12).
//
// `LfxDef` is driven as the `&dyn EffectDef` the render path will drive it as,
// over a real bundle in a real second process. The unit cases in
// `lumit-lfx/src/def.rs` prove the marshalling against a fake; these two prove
// that the wiring `LfxDef::hosted` does - the driver, the gate over it, the
// broker behind both - reaches a plugin and comes back.
// ---------------------------------------------------------------------------

/// One described personality as the catalogue entry it becomes, over a broker
/// this test owns.
fn a_definition(broker: Broker, personality: Personality) -> Option<lumit_lfx::LfxDef> {
    let descriptor =
        lumit_lfx::describe::PluginDescriptor::from(described(&broker, personality)?.clone());
    let schema = Box::leak(Box::new(
        lumit_lfx::schema::schema_of(&descriptor).expect("the plugin lowers to an effect"),
    ));
    // The bundle's own lock, which every definition built over one broker
    // shares and which is armed from **every** plugin the bundle described:
    // `lfx.thread-unsafe` serialises the bundle rather than the plugin
    // (docs/impl/lfx.md §4.4), so a bundle where one personality declared it
    // would pin the rest of them too. This one declares nothing of the sort, so
    // the pool is free to grow and never takes the lock.
    let serial =
        lumit_lfx::Serial::for_bundle(broker.described().iter().map(|plugin| plugin.traits.flags));
    Some(lumit_lfx::LfxDef::hosted(
        &descriptor,
        schema,
        std::sync::Arc::new(std::sync::Mutex::new(broker)),
        &serial,
    ))
}

/// The whole road: a bag resolved by the engine becomes the dense value array
/// a plugin in another process reads, and the picture it paints comes back
/// through the definition into the caller's own buffer.
///
/// The Full personality multiplies every sample by its first control, so the
/// number the bag carried is visible in the answer rather than merely accepted.
#[test]
fn a_described_plugin_renders_through_the_definition() {
    use lumit_core::fx::{EffectDef, ParamId, Params, Value};

    let test = "a_described_plugin_renders_through_the_definition";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let Some(def) = a_definition(broker, Personality::Full) else {
        skipped(test);
        return;
    };
    assert_eq!(def.schema().match_name, "lfx:org.lumit.testplug.full");
    assert_eq!(def.identifier(), "org.lumit.testplug.full");

    let bag = [(ParamId::new("gain"), Value::Float(0.5))];
    let mut rgba = vec![0.4_f32; (FRAME.0 * FRAME.1 * 4) as usize];
    def.apply_cpu(&mut rgba, FRAME.0, FRAME.1, Params::new(&bag));

    assert_eq!(def.last_error(), None, "nothing went wrong");
    assert!(
        rgba.iter().all(|sample| (sample - 0.2).abs() < 1e-6),
        "the plugin's own maths, with the bag's number in it: {:?}",
        rgba.first()
    );
}

/// And a frame that never came back leaves the caller's buffer exactly as the
/// definition found it - identity byte for byte, with the host's own sentence
/// filed for the badge (§14 item 7).
///
/// The plugin hangs, so the frame is refused at the deadline rather than
/// answered; what this asks is what `LfxDef` does with that, which is nothing
/// at all to the picture.
#[test]
fn a_frame_that_never_came_back_leaves_the_definitions_picture_alone() {
    use lumit_core::fx::{EffectDef, Params};

    let test = "a_frame_that_never_came_back_leaves_the_definitions_picture_alone";
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(mut broker) = a_broker(root.path(), &[(HANG_ENV, "1")]) else {
        skipped(test);
        return;
    };
    broker.describe().expect("a description");
    let Some(def) = a_definition(broker, Personality::Hang) else {
        skipped(test);
        return;
    };

    let before: Vec<f32> = (0..(FRAME.0 * FRAME.1 * 4))
        .map(|sample| f32::from(sample as u16) / 256.0)
        .collect();
    let mut rgba = before.clone();
    def.apply_cpu(&mut rgba, FRAME.0, FRAME.1, Params::EMPTY);

    let bits: Vec<u32> = rgba.iter().map(|sample| sample.to_bits()).collect();
    let was: Vec<u32> = before.iter().map(|sample| sample.to_bits()).collect();
    assert_eq!(bits, was, "identity, byte for byte");
    assert!(
        def.last_error().is_some(),
        "and the layer has a sentence to wear"
    );
}
