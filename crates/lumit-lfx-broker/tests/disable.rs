//! The first of the three places a disable reaches, driven through a real
//! second process (docs/impl/lfx.md §5.4 place 1).
//!
//! **One test, and its own binary.** `lumit_lfx::discover`'s switched-off list
//! is one table for the whole process - which is the point of it - so a case
//! that ticks a box in it cannot share a binary with the folder-of-bundles
//! scan beside it without the two reading each other's ticks. Cargo gives each
//! file in `tests/` a process of its own, which is the cheapest lock there is.
//!
//! What is under test is the property the scan promises and nothing short of a
//! broker can show: the list a broker is spawned with is a **share** of the
//! running table, so a tick landing after `Broker::spawn` has returned - after
//! the handshake, after the `Manifest` round trip - is still read by the
//! `Describe` that has not happened yet, and the plugin it names is never
//! asked what it is. `lumit-lfx`'s own
//! `the_disable_list_a_broker_is_spawned_with_is_a_share_of_the_running_table`
//! pins the handle under it; this pins the answer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lumit_budget::Ledger;
use lumit_lfx::bundle::{BUNDLE_SUFFIX, PAYLOAD_EXTENSION};
use lumit_lfx::ipc::proto::PixelDepth;
use lumit_lfx::ipc::ring::RingPlan;
use lumit_lfx::manifest::{CONTENTS_DIR, FAMILY_NAMES, MANIFEST_FILE};
use lumit_lfx::{discover, Broker, BrokerConfig};
use lumit_lfx_testplug::{Personality, PERSONALITIES};

/// The frame the ring is sized from. Small on purpose: the property here is
/// about a list, not about memory.
const FRAME: (u32, u32) = (8, 8);

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

/// The id a personality declares, as text.
fn id_of(personality: Personality) -> String {
    personality.id().to_string_lossy().into_owned()
}

/// The listing a well-formed bundle of the test plugin carries, written from
/// the fixture's own table rather than by hand: the re-check the describe runs
/// compares the two records field for field, so a hand-written listing would
/// test the typist.
fn a_manifest() -> String {
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
                    .map_or_else(
                        || {
                            panic!(
                                "the fixture declares category {declared}, which \
                                 FAMILY_NAMES cannot spell"
                            )
                        },
                        |(name, _)| (*name).to_owned(),
                    )
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
        text.push_str(&format!(
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
        ));
    }
    text
}

/// Lay the test plugin out as a bundle this machine's `payload` will find.
fn a_bundle_in(root: &Path, source: &Path) -> PathBuf {
    let bundle = root.join(format!("Good{BUNDLE_SUFFIX}"));
    let arch = bundle
        .join(CONTENTS_DIR)
        .join(lumit_lfx::bundle::arch_dirs()[0]);
    std::fs::create_dir_all(&arch).expect("the bundle directory");
    std::fs::write(bundle.join(CONTENTS_DIR).join(MANIFEST_FILE), a_manifest())
        .expect("the listing");
    std::fs::copy(source, arch.join(format!("Good.{PAYLOAD_EXTENSION}"))).expect("the payload");
    bundle
}

#[test]
fn a_plugin_switched_off_after_the_broker_starts_is_still_switched_off_at_describe() {
    let test = "a_plugin_switched_off_after_the_broker_starts_is_still_switched_off_at_describe";
    let Some(source) = built_cdylib() else {
        eprintln!(
            "{test}: skipped - {} was not found in the target directory. \
             Build it first: cargo build -p lumit-lfx-testplug",
            cdylib_name()
        );
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory");
    let bundle = a_bundle_in(root.path(), &source);
    let payload = lumit_lfx::bundle::payload(&bundle).expect("a build for this machine");

    discover::set_disabled(&BTreeSet::new());
    let late = id_of(Personality::Full);

    let mut config = BrokerConfig::new(
        &bundle,
        payload,
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F16),
    );
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    // Exactly what `scan_bundle` hands `Broker::spawn`: a share of the running
    // table and not a copy of it, taken before the tick.
    config.disabled = discover::running_list();

    let ledger = Ledger::new();
    let mut broker = Broker::spawn(config, &ledger).expect("a broker");

    // The listing first, always - it runs none of the bundle's code, and it is
    // the round trip the tick below has to land after to mean anything.
    let listed: Vec<String> = broker
        .manifest()
        .expect("the listing")
        .iter()
        .map(|identity| identity.id.clone())
        .collect();
    assert!(
        listed.contains(&late),
        "the bundle declares the plugin this case switches off: {listed:?}"
    );

    // The tick, after the spawn and after the handshake, and before anything
    // has opened the module.
    discover::set_enabled(&late, false);

    let described: Vec<String> = broker
        .describe()
        .expect("the describe")
        .iter()
        .map(|plugin| plugin.identity.id.clone())
        .collect();
    assert!(
        !described.contains(&late),
        "a tick landing between the spawn and the describe reaches it: {described:?}"
    );
    assert!(
        described.len() > 1,
        "and it costs the rest of the bundle nothing: {described:?}"
    );
    assert!(
        !broker.refused().iter().any(|(id, _)| *id == late),
        "a plugin nobody asked is not a plugin that said no"
    );

    // And back on, with the same broker: a second describe asks the module it
    // has already opened, and the plugin is there.
    discover::set_enabled(&late, true);
    let again: Vec<String> = broker
        .describe()
        .expect("the second describe")
        .iter()
        .map(|plugin| plugin.identity.id.clone())
        .collect();
    assert!(
        again.contains(&late),
        "switching it back on is just as live: {again:?}"
    );
}
