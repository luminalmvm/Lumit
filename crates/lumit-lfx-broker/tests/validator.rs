//! `lfx-validator` over a real bundle in a real second process
//! (docs/impl/lfx.md §9, §12).
//!
//! # In plain terms
//!
//! The validator's own crate can prove its arithmetic - the table's shape, the
//! seeded edges, what a moved pixel does to a digest - with no plugin at all.
//! What it cannot prove there is the only thing that matters: that the ten
//! suites, driven end to end against twelve personalities across a pipe and a
//! ring, come back saying the right things about each of them.
//!
//! So this file lives here, for the flat Cargo reason the rest of this
//! directory does: `CARGO_BIN_EXE_lumit-lfx-broker` exists only inside the
//! package that owns the binary, and the validator opens nothing in this
//! process - there is no in-process road it could take instead.
//!
//! **One test drives the whole bundle** and the rest read what it found,
//! because a validator run is ten brokers and five hundred frames and running
//! it once per assertion would be running it five times. The bundle is staged
//! at a fixed place under `target/` rather than in a temporary directory, so
//! that CI's next step can point the shipped program at the same bundle this
//! one drove.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use lumit_lfx::manifest::{CONTENTS_DIR, FAMILY_NAMES, MANIFEST_FILE};
use lumit_lfx_testplug::{Personality, PERSONALITIES};
use lumit_lfx_validator::{validate, Baseline, Finding, Options, Outcome, Report, Suite, ACTIONS};

/// The personalities that are supposed to be refused at describe, and which
/// therefore never reach a suite that drives a frame.
///
/// Each is refused for a different reason and all three are refusals the host
/// files rather than the validator: a describe that answered `false`, two
/// controls on one id, and an extension version 1 does not offer.
const DELIBERATELY_BROKEN: [Personality; 3] = [
    Personality::BrokenDescribe,
    Personality::DuplicateIds,
    Personality::MissingExtension,
];

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
fn arch_dir() -> &'static str {
    lumit_lfx::bundle::arch_dirs()
        .first()
        .copied()
        .unwrap_or("")
}

/// The target directory, found by walking up from this test binary.
fn target_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("target"))
}

/// Where the staged bundle lands, and where CI's next step looks for it.
fn bundle_dir() -> PathBuf {
    target_dir().join("lfx-validator-bundle")
}

/// Where the table is written, so CI can put it in the pull request body.
fn table_path() -> PathBuf {
    if let Some(named) = std::env::var_os("LFX_CONFORMANCE_OUT") {
        return PathBuf::from(named);
    }
    target_dir().join("lfx-conformance.md")
}

/// The id a personality declares, as text.
fn id_of(personality: Personality) -> String {
    personality.id().to_string_lossy().into_owned()
}

/// The manifest a well-formed bundle of the test plugin carries, written from
/// the fixture's own table so that a hand-written listing cannot test the
/// typist.
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

/// Lay the test plugin out as a real bundle at a fixed place under `target/`,
/// and answer with the bundle directory.
///
/// **Staged once for the whole binary.** Cargo runs these cases on threads of
/// one process, and a case that re-laid the bundle would be pulling the payload
/// out from under another case's broker. It is also why the staging is a fixed
/// path rather than a temporary one: CI's next step runs the shipped program
/// over the bundle this file drove.
///
/// **`None` means one thing only**: the fixture cdylib was not built, which is
/// the one reason a case in this file may skip.
fn a_staged_bundle() -> Option<&'static PathBuf> {
    static ONCE: OnceLock<Option<PathBuf>> = OnceLock::new();
    ONCE.get_or_init(|| {
        let source = built_cdylib()?;
        let root = bundle_dir();
        let bundle = root.join("Test.lfx.bundle");
        let contents = bundle.join(CONTENTS_DIR);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(contents.join(arch_dir())).expect("the bundle directory");
        std::fs::write(contents.join(MANIFEST_FILE), a_manifest()).expect("the manifest");
        std::fs::copy(&source, contents.join(arch_dir()).join("Test.lfx")).expect("the payload");
        Some(bundle)
    })
    .as_ref()
}

/// The options a case runs under: the staged bundle, the broker this package
/// owns, and deadlines short enough that a hung plugin is a failed test rather
/// than a stopped one.
fn options_over(bundle: &Path) -> Options {
    let mut options = Options::over(bundle).expect("the staged bundle has a payload");
    options.broker_exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    options.control_timeout = Duration::from_secs(10);
    options.process_timeout = Duration::from_secs(10);
    options
}

/// The one validator run every case in this file reads.
///
/// `None` is the fixture not being built. The run also writes the table where
/// CI expects it, because the run and the table are one thing.
fn the_run() -> Option<&'static Report> {
    static ONCE: OnceLock<Option<Report>> = OnceLock::new();
    ONCE.get_or_init(|| {
        let bundle = a_staged_bundle()?;
        let report = validate(&options_over(bundle)).expect("the bundle opens");
        let table = report.table();
        eprintln!("{table}");
        let out = table_path();
        if let Err(why) = std::fs::write(&out, &table) {
            eprintln!("the table could not be written to {}: {why}", out.display());
        }
        Some(report)
    })
    .as_ref()
}

/// Say why a test did nothing, by name, so a skip is never silent.
fn skipped(test: &str) {
    eprintln!(
        "{test}: skipped - {} was not found in the target directory. \
         Build it first: cargo build -p lumit-lfx-testplug",
        cdylib_name()
    );
}

/// Every row about one plugin.
fn rows_for<'a>(report: &'a Report, plugin: &str) -> Vec<&'a lumit_lfx_validator::Row> {
    report
        .rows
        .iter()
        .filter(|row| row.plugin == plugin)
        .collect()
}

// --------------------------------------------------------------- the tests --

/// The nine honest personalities pass all ten suites, which is the whole of
/// what `lfx-conformance` gates on.
///
/// The three that are supposed to fail are named here and are not asserted
/// green - but nor are they merely excluded: the next test requires each of
/// them to have been refused, so a bundle that quietly started catalouging a
/// plugin with two controls on one id would fail there rather than pass here.
#[test]
fn every_suite_passes_over_the_fixtures_honest_personalities() {
    let test = "every_suite_passes_over_the_fixtures_honest_personalities";
    let Some(report) = the_run() else {
        skipped(test);
        return;
    };
    let broken: Vec<String> = DELIBERATELY_BROKEN.iter().copied().map(id_of).collect();

    for personality in PERSONALITIES {
        let id = id_of(personality);
        if broken.contains(&id) {
            continue;
        }
        let rows = rows_for(report, &id);
        assert!(
            !rows.is_empty(),
            "{id} has no row at all in:\n{}",
            report.table()
        );
        for row in &rows {
            assert!(
                !row.outcome.is_refusal(),
                "{id} was refused by the {} suite: {}\n{}",
                row.suite.name(),
                row.outcome.sentence(),
                report.table()
            );
        }
        // Every question was put to this plugin, the baseline among them: it is
        // skipped when no baseline was named, which is this run, and a skip is
        // a row. A suite that filed nothing at all is the fault being caught.
        // `Suite::Bundle` is not one of the ten - it is where the bundle's own
        // answers go - so it is the one heading a plugin need not carry.
        for suite in Suite::ALL.into_iter().filter(|s| *s != Suite::Bundle) {
            assert!(
                rows.iter().any(|row| row.suite == suite),
                "{id} was never asked the {} question:\n{}",
                suite.name(),
                report.table()
            );
        }

        // The lifecycle row's sentence is built from the steps that answered,
        // so a step the suite stopped driving stops being named here. This is
        // what holds `ACTIONS` to the order actually driven rather than to a
        // copy of itself.
        let lifecycle = rows
            .iter()
            .find(|row| row.suite == Suite::Lifecycle)
            .map(|row| row.outcome.sentence())
            .unwrap_or_default();
        for action in ACTIONS {
            assert!(
                lifecycle.contains(action),
                "{id}'s lifecycle row never drove {action}: {lifecycle}"
            );
        }
    }

    assert_eq!(
        report.described,
        PERSONALITIES.len() - DELIBERATELY_BROKEN.len(),
        "the wrong number of effects described:\n{}",
        report.table()
    );
    assert_eq!(
        report.listed,
        PERSONALITIES.len(),
        "the listing is read with the module shut and holds every personality:\n{}",
        report.table()
    );
}

/// Each of the three broken personalities reaches the table as the host's own
/// typed refusal, and nothing was driven for it.
///
/// The point is the pairing: a refusal is a **row** rather than an absence
/// (§4.3), so a vendor reads why their effect did not appear instead of
/// wondering whether it was switched off.
#[test]
fn a_plugin_the_host_refuses_reaches_the_table_as_a_named_refusal() {
    let test = "a_plugin_the_host_refuses_reaches_the_table_as_a_named_refusal";
    let Some(report) = the_run() else {
        skipped(test);
        return;
    };
    for personality in DELIBERATELY_BROKEN {
        let id = id_of(personality);
        let rows = rows_for(report, &id);
        assert!(
            rows.iter().any(|row| matches!(
                &row.outcome,
                Outcome::Refused(Finding::RefusedAtDescribe(_))
            )),
            "{id} should be refused at describe:\n{}",
            report.table()
        );
        assert!(
            rows.iter()
                .all(|row| matches!(row.suite, Suite::Describe | Suite::Lifecycle)),
            "{id} was driven after it was refused:\n{}",
            report.table()
        );
    }
    assert!(
        !report.passed(),
        "a bundle with three broken effects in it is not a bundle that passed"
    );
}

/// A run told which plugins to expect a refusal from passes, and the same flag
/// naming a plugin that was fine fails.
///
/// This is what `lfx-conformance` runs the shipped program with, and it has to
/// be an assertion rather than a way of turning the tool off: a vendor who
/// listed every plugin they had would get a red run, not a green one.
#[test]
fn a_bundle_whose_refusals_were_expected_passes_and_an_unmet_expectation_does_not() {
    let test = "a_bundle_whose_refusals_were_expected_passes_and_an_unmet_expectation_does_not";
    let Some(bundle) = a_staged_bundle() else {
        skipped(test);
        return;
    };

    let mut expected = options_over(bundle);
    expected.allow_refused = DELIBERATELY_BROKEN.iter().copied().map(id_of).collect();
    let met = validate(&expected).expect("the bundle opens");
    assert!(
        met.passed(),
        "a bundle whose only refusals were expected should pass:\n{}",
        met.table()
    );
    assert!(
        met.rows
            .iter()
            .any(|row| matches!(row.outcome, Outcome::Expected(_))),
        "the expected refusals should be lines rather than absences:\n{}",
        met.table()
    );

    let mut unmet = options_over(bundle);
    unmet.allow_refused = vec![id_of(Personality::Passthrough)];
    let missed = validate(&unmet).expect("the bundle opens");
    assert!(
        missed.rows.iter().any(|row| {
            row.plugin == id_of(Personality::Passthrough)
                && row.outcome == Outcome::Refused(Finding::ExpectedARefusal)
        }),
        "a plugin expected to be refused and which was not is a refusal of its own:\n{}",
        missed.table()
    );
}

/// The stored digest is the only pressure on the one obligation nothing can
/// enforce: a frame that moved while the version stood still is a refusal.
///
/// The first run writes what the frames are and says so - a run that created a
/// record must not print "run again with --write-baseline" at the vendor who
/// just did. The record is then edited to say the **fp16** frame was something
/// else, which is exactly what a vendor who rewrote their `apply_f16` maths and
/// left the version standing looks like from the file's side, and the second
/// run refuses by name and says which depth moved.
#[test]
fn the_baseline_refuses_a_frame_that_moved_with_no_version_bump() {
    let test = "the_baseline_refuses_a_frame_that_moved_with_no_version_bump";
    let Some(bundle) = a_staged_bundle() else {
        skipped(test);
        return;
    };
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let path = root.path().join("lfx-baseline.json");

    let mut writing = options_over(bundle);
    writing.baseline = Some(path.clone());
    writing.write_baseline = true;
    let written = validate(&writing).expect("the bundle opens");
    assert!(
        path.is_file(),
        "the baseline should have been written:\n{}",
        written.table()
    );
    assert!(
        written.rows.iter().any(|row| {
            row.suite == Suite::Baseline
                && matches!(row.outcome, Outcome::Noted(Finding::BaselineWritten { .. }))
        }),
        "the run that wrote the record should say what it wrote:\n{}",
        written.table()
    );
    assert!(
        !written
            .rows
            .iter()
            .any(|row| matches!(row.outcome, Outcome::Noted(Finding::NoStoredBaseline))),
        "a run that wrote a record must not ask for one:\n{}",
        written.table()
    );

    // Read back unchanged: the frames are what they were a moment ago.
    let mut comparing = options_over(bundle);
    comparing.baseline = Some(path.clone());
    let same = validate(&comparing).expect("the bundle opens");
    assert!(
        same.rows.iter().any(|row| {
            row.suite == Suite::Baseline && matches!(row.outcome, Outcome::Passed(_))
        }),
        "an unchanged bundle should match its own baseline:\n{}",
        same.table()
    );

    // Now say the fp16 frame used to be something else, at the same version,
    // and leave the fp32 entry alone: a record of one depth would pass this.
    let mut record = Baseline::read(&path).expect("the baseline reads");
    for stored in record.plugins.values_mut() {
        stored.digests.fp16 = "0000000000000000".to_owned();
    }
    record.write(&path).expect("the baseline writes");

    let moved = validate(&comparing).expect("the bundle opens");
    assert!(
        moved.rows.iter().any(|row| {
            matches!(
                &row.outcome,
                Outcome::Refused(Finding::PixelsMovedWithNoVersionBump { depth, .. })
                    if *depth == "fp16"
            )
        }),
        "an fp16 frame that moved with no version bump should be refused by name:\n{}",
        moved.table()
    );
    assert!(!moved.passed());
}
