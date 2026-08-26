//! The scan that turns a folder of bundles into effects (docs/12 §2.6).
//!
//! One test, deliberately: [`lumit_ofx::discover`] keeps what it has registered
//! in a process-wide table — that is what makes a rescan idempotent — so two
//! test functions scanning the same fixture in the same binary would be reading
//! each other's registrations. Everything the package promises about a scan is
//! therefore asserted in one run, in the order it happens.
//!
//! Hosted in-process ([`Hosting::InProcess`]): whether a folder of bundles
//! becomes the right set of catalogue entries is a question about the scan, and
//! the broker's own behaviour is tested in the broker's crate, which is the only
//! place `CARGO_BIN_EXE_…` names its executable.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lumit_core::fx::EffectDef;
use lumit_ofx::bundle::BUNDLE_ARCH_DIR;
use lumit_ofx::discover::{self, Hosting, ScanOptions};

/// The test plugin's file name on this platform.
fn test_plugin_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_ofx_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_ofx_testplug.dylib"
    } else {
        "liblumit_ofx_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
fn test_plugin() -> Option<PathBuf> {
    let name = test_plugin_file_name();
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

/// Lay the test plugin out as a real bundle called `name` inside `root`.
fn a_bundle_in(root: &Path, name: &str) -> Option<PathBuf> {
    let source = test_plugin()?;
    let dir = root
        .join(format!("{name}.ofx.bundle"))
        .join("Contents")
        .join(BUNDLE_ARCH_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    let binary = dir.join(format!("{name}.ofx"));
    std::fs::copy(&source, &binary).ok()?;
    Some(binary)
}

/// A bundle whose binary is not a library at all — the broken installer.
fn an_unloadable_bundle_in(root: &Path, name: &str) -> Option<PathBuf> {
    let dir = root
        .join(format!("{name}.ofx.bundle"))
        .join("Contents")
        .join(BUNDLE_ARCH_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    let binary = dir.join(format!("{name}.ofx"));
    std::fs::write(&binary, b"this is not a shared library").ok()?;
    Some(binary)
}

/// A catalogue that records rather than registers, with the real one's rule:
/// a name it already answers to is refused.
#[derive(Default)]
struct Recorder {
    names: Vec<String>,
}

impl Recorder {
    fn take(&mut self, def: &'static dyn EffectDef) -> bool {
        let name = def.schema().match_name.to_owned();
        if self.names.contains(&name) {
            return false;
        }
        self.names.push(name);
        true
    }
}

#[test]
fn a_folder_of_bundles_becomes_exactly_the_effects_it_should() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(_good) = a_bundle_in(root.path(), "Good") else {
        eprintln!(
            "a_folder_of_bundles_becomes_exactly_the_effects_it_should: skipped — {} was not \
             found. Build it first: cargo build -p lumit-ofx-testplug",
            test_plugin_file_name()
        );
        return;
    };
    an_unloadable_bundle_in(root.path(), "Broken").expect("a file can be written");

    // The one the user has switched off. It is an identifier the good bundle
    // really declares, so the assertion below is about the preference and not
    // about a name nothing answers to.
    let switched_off = "com.lumitlab.testplug.identity";
    let mut disabled = BTreeSet::new();
    disabled.insert(switched_off.to_owned());

    // **The env-var path is honoured**: nothing is passed in `paths` but the
    // fixture folder in `OFX_PLUGIN_PATH`, so anything found came through it.
    // (Set here rather than around a narrower call because `search_paths`
    // reads the environment of the whole process.)
    std::env::set_var("OFX_PLUGIN_PATH", root.path());
    let mut options = ScanOptions::standard();
    options.hosting = Hosting::InProcess;
    options.disabled = disabled;
    assert!(
        lumit_ofx::bundle::search_paths().contains(&root.path().to_path_buf()),
        "OFX_PLUGIN_PATH is one of the standard search paths (docs/12 §2.6)"
    );
    // The scan itself is then pointed at the fixture alone. `standard()` would
    // also sweep the machine's real OFX folder, and a test whose expected list
    // depends on what the developer happens to have installed is no test —
    // this one found a hundred and eighty HitFilm plugins the first time it ran.
    options.paths = vec![root.path().to_path_buf()];

    let mut catalogue = Recorder::default();
    let first = discover::scan(&options, &mut |def| catalogue.take(def));

    // Exactly the plugins that describe, minus the switched-off one, and one
    // entry for the identifier the bundle declares twice at two versions.
    let registered: Vec<&str> = first
        .registered
        .iter()
        .map(|found| found.match_name.as_str())
        .collect();
    assert_eq!(
        registered,
        vec![
            "ofx:com.lumitlab.testplug",
            "ofx:com.lumitlab.testplug.passthrough",
            "ofx:com.lumitlab.testplug.unsafe",
        ],
        "the disabled plugin, the two that will not describe and the one with \
         colliding parameter ids are all absent"
    );

    // The plugin's own facts came with it, so the browser can place it.
    let full = first
        .registered
        .iter()
        .find(|found| found.identifier == "com.lumitlab.testplug")
        .expect("the full plugin registered");
    assert_eq!(full.label, "Test plug");
    assert_eq!(full.grouping, "Lumit/Test");

    // **Every refusal is a line, and none of them is a dialogue.**
    let report = first.skipped.join("\n");
    assert!(
        report.contains("Broken.ofx") && report.contains("switched off in preferences"),
        "the unloadable bundle and the disabled plugin each said why: {report}"
    );
    assert!(
        report.contains(switched_off),
        "the switched-off plugin is named in the report: {report}"
    );
    assert!(
        !first
            .skipped
            .iter()
            .any(|line| line.contains("com.lumitlab.testplug.passthrough")),
        "a plugin that registered is not also reported as skipped"
    );

    // **A second scan registers nothing again** — it is a rescan, not a second
    // catalogue.
    let second = discover::scan(&options, &mut |def| catalogue.take(def));
    assert!(
        second.registered.is_empty(),
        "a rescan found {:?} all over again",
        second.registered
    );
    assert_eq!(
        catalogue.names.len(),
        3,
        "the catalogue took each name exactly once"
    );

    // And the session's own table agrees with what the scans said.
    let names: Vec<String> = discover::discovered()
        .into_iter()
        .map(|found| found.match_name)
        .collect();
    for name in &registered {
        assert!(names.contains(&(*name).to_owned()), "{name} is remembered");
    }
    assert_eq!(
        discover::plugin_of("ofx:com.lumitlab.testplug")
            .map(|found| found.grouping)
            .as_deref(),
        Some("Lumit/Test"),
    );
    assert!(discover::plugin_of("ofx:nothing.of.the.sort").is_none());

    // Switching one off at run time is answered immediately, which is what the
    // badge is drawn from.
    assert!(!discover::is_disabled("com.lumitlab.testplug"));
    discover::set_enabled("com.lumitlab.testplug", false);
    assert!(discover::is_disabled("com.lumitlab.testplug"));
    discover::set_enabled("com.lumitlab.testplug", true);
    assert!(!discover::is_disabled("com.lumitlab.testplug"));

    std::env::remove_var("OFX_PLUGIN_PATH");
}
