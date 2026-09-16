//! The template repository, built and validated before it is published
//! (docs/impl/lfx.md §10, §12).
//!
//! # In plain terms
//!
//! `template/` is the repository a vendor starts from: the header, the three
//! sets of bindings, and one working example per language. It is published
//! separately and staged in-tree so that this workspace's own CI is what says
//! whether it works - a template that has drifted from the ABI is worse than
//! no template at all, because it is wrong in a way that looks official.
//!
//! So this file asks the two questions the note asks of it.
//!
//! **Is it the same ABI?** The header and the Rust mirror in the template are
//! *copies*, and the only thing that keeps a copy honest is a test that reads
//! both. They are compared byte for byte, and the examples' listings are held
//! to the ABI version this workspace declares.
//!
//! **Does it work?** Each example is compiled here, with this machine's own
//! compilers, laid out as a bundle, and driven through `lfx-validator` - the
//! same ten suites over a real second process that the twelve fixture
//! personalities go through next door in `validator.rs`. An example that
//! passes nine suites and stumbles on the tenth is an example that teaches
//! somebody the tenth mistake.
//!
//! It lives in this package for the flat Cargo reason the rest of this
//! directory does: `CARGO_BIN_EXE_lumit-lfx-broker` exists only inside the
//! package that owns the binary, and the validator opens nothing in this
//! process.
//!
//! **The toolchain is the one thing that may be absent.** A machine with no
//! CMake, no C compiler or no Python cannot build the template, and that is a
//! skip rather than a failure - said by name, never silently, and never on the
//! three platforms the template's own CI runs on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use lumit_lfx_validator::{validate, Options, Report};

/// The three examples, as `(bundle name, the directory the example lives in)`.
///
/// One per set of bindings: the C header on its own, the C++ wrapper over it,
/// and the Rust pair. A language the template ships bindings for and does not
/// build is a language nothing holds to the ABI.
const EXAMPLES: [(&str, &str); 3] = [
    ("Exposure", "exposure"),
    ("Vignette", "vignette"),
    ("Saturation", "saturation"),
];

/// The repository root, from this package's own manifest.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .expect("the workspace root is two directories above this crate")
}

/// The template repository, staged in-tree.
fn template() -> PathBuf {
    repository().join("template")
}

/// The target directory, found by walking up from this test binary.
fn target_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("target"))
}

/// Where the template is built, and where CI's next step looks for the
/// bundles.
fn build_dir() -> PathBuf {
    target_dir().join("lfx-template")
}

/// The architecture directory this machine's payload goes in - the host's own
/// answer, passed to the template's build rather than guessed twice.
fn arch_dir() -> &'static str {
    lumit_lfx::bundle::arch_dirs()
        .first()
        .copied()
        .unwrap_or("")
}

/// Say why a test did nothing, by name, so a skip is never silent.
fn skipped(test: &str, why: &str) {
    eprintln!("{test}: skipped - {why}");
}

/// Run a program, answering its output, or `None` where it is not on this
/// machine at all.
fn run(program: &str, arguments: &[&str]) -> Option<std::process::Output> {
    match Command::new(program).args(arguments).output() {
        Ok(output) => Some(output),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => None,
        Err(why) => panic!("{program} could not be run: {why}"),
    }
}

/// Fail with the program's own words, which is the only useful thing to say
/// about a compiler that would not compile something.
fn must_have_worked(what: &str, output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{what} failed ({}):\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The name Cargo gives the Rust example's payload on this platform.
fn rust_payload() -> &'static str {
    if cfg!(target_os = "windows") {
        "saturation.dll"
    } else if cfg!(target_os = "macos") {
        "libsaturation.dylib"
    } else {
        "libsaturation.so"
    }
}

/// Build every example and lay each one out as a bundle, once for the whole
/// binary.
///
/// `Err` is a toolchain this machine has not got, which is the one reason a
/// case in this file may skip; a build that ran and failed panics here rather
/// than skipping, because that is the template being broken.
fn the_built_bundles() -> Result<&'static Vec<PathBuf>, &'static str> {
    static ONCE: OnceLock<Result<Vec<PathBuf>, &'static str>> = OnceLock::new();
    ONCE.get_or_init(|| {
        let template = template();
        let build = build_dir();
        let bundles = build.join("bundles");
        let _ = std::fs::remove_dir_all(&build);

        // The C and C++ examples, through the template's own CMakeLists -
        // which is the build a vendor runs, so a broken one is caught here
        // rather than in somebody's first afternoon.
        let configure = run(
            "cmake",
            &[
                "-S",
                &template.display().to_string(),
                "-B",
                &build.join("cmake").display().to_string(),
                "-DCMAKE_BUILD_TYPE=Release",
                &format!("-DLFX_ARCH_DIR={}", arch_dir()),
                &format!("-DLFX_BUNDLE_DIR={}", bundles.display()),
            ],
        );
        let Some(configure) = configure else {
            return Err("cmake is not on this machine");
        };
        must_have_worked("configuring the template", &configure);
        let built = run(
            "cmake",
            &[
                "--build",
                &build.join("cmake").display().to_string(),
                "--config",
                "Release",
            ],
        )
        .ok_or("cmake is not on this machine")?;
        must_have_worked("building the template's C and C++ examples", &built);

        // The Rust example, through Cargo, into a target directory of its own:
        // the template is a workspace of its own and must stay one.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let rust_target = build.join("rust");
        let compiled = run(
            &cargo,
            &[
                "build",
                "--release",
                "--manifest-path",
                &template.join("Cargo.toml").display().to_string(),
                "--target-dir",
                &rust_target.display().to_string(),
            ],
        )
        .ok_or("cargo is not on this machine")?;
        must_have_worked("building the template's Rust example", &compiled);

        // And laid out by the template's own staging script, for the reason
        // the CMakeLists is used above: the path a vendor takes is the path
        // that gets tested.
        let staged = run(
            "python3",
            &[
                &template
                    .join("scripts/stage-bundle.py")
                    .display()
                    .to_string(),
                "--payload",
                &rust_target
                    .join("release")
                    .join(rust_payload())
                    .display()
                    .to_string(),
                "--manifest",
                &template
                    .join("examples/saturation/lfx.toml")
                    .display()
                    .to_string(),
                "--name",
                "Saturation",
                "--out",
                &bundles.display().to_string(),
                "--arch",
                arch_dir(),
            ],
        );
        let Some(staged) = staged else {
            return Err("python3 is not on this machine");
        };
        must_have_worked("staging the template's Rust example as a bundle", &staged);

        Ok(EXAMPLES
            .iter()
            .map(|(name, _)| bundles.join(format!("{name}.lfx.bundle")))
            .collect())
    })
    .as_ref()
    .map_err(|why| *why)
}

/// One validator run over one bundle, with the broker this package owns.
fn the_run_over(bundle: &Path) -> Report {
    let mut options = Options::over(bundle).unwrap_or_else(|why| {
        panic!(
            "{} holds no payload this machine can open: {why}",
            bundle.display()
        )
    });
    options.broker_exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-lfx-broker")));
    options.control_timeout = Duration::from_secs(10);
    options.process_timeout = Duration::from_secs(10);
    validate(&options).unwrap_or_else(|why| panic!("{} would not open: {why}", bundle.display()))
}

// --------------------------------------------------------------- the tests --

/// The template's header and its Rust mirror are this workspace's own, byte
/// for byte.
///
/// They are copies, and a copy is only as honest as the test that reads both.
/// A field appended to `lfx.h` here and not there would leave a vendor
/// compiling against an ABI the host stopped speaking, and nothing at run time
/// could tell them so: every struct's size prefix would be the *older* number,
/// which is exactly the case the growth rule makes legal.
#[test]
fn the_templates_copy_of_the_abi_is_this_workspaces_own() {
    let workspace = repository().join("crates/lumit-lfx-abi");
    let template = template();
    for (canonical, copy) in [
        (
            workspace.join("include/lfx.h"),
            template.join("include/lfx.h"),
        ),
        (
            workspace.join("src/lib.rs"),
            template.join("rust/lfx-sys/src/abi.rs"),
        ),
        (workspace.join("LICENSE"), template.join("LICENSE")),
    ] {
        let original = std::fs::read(&canonical)
            .unwrap_or_else(|why| panic!("{} could not be read: {why}", canonical.display()));
        let published = std::fs::read(&copy)
            .unwrap_or_else(|why| panic!("{} could not be read: {why}", copy.display()));
        assert_eq!(
            original,
            published,
            "{} has drifted from {}. Copy it across: the template publishes this \
             workspace's own declarations, never a reading of them.",
            copy.display(),
            canonical.display()
        );
    }
}

/// Every example's listing declares the ABI version this workspace speaks.
///
/// The listing is read with the module shut, so a bundle that named another
/// version would be refused before any of its code ran - and a template whose
/// examples were refused on sight is the worst of all the ways to be wrong.
#[test]
fn every_example_declares_the_abi_version_this_workspace_speaks() {
    let declared = format!("abi_version = {}", lumit_lfx_abi::LFX_ABI_VERSION);
    for (name, directory) in EXAMPLES {
        let manifest = template().join("examples").join(directory).join("lfx.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|why| panic!("{} could not be read: {why}", manifest.display()));
        assert!(
            text.lines().any(|line| line.trim() == declared),
            "{name}'s listing does not say `{declared}`:\n{text}"
        );
    }
}

/// Every example builds with this machine's own compilers and passes every
/// suite `lfx-validator` has.
///
/// The three between them are one per set of bindings the template ships, and
/// each is driven exactly as a vendor's own bundle would be: laid out as a
/// bundle, opened by a broker of its own, and asked the ten questions. A
/// refusal here is the template teaching somebody a mistake.
#[test]
fn every_example_builds_and_passes_every_suite() {
    let test = "every_example_builds_and_passes_every_suite";
    let bundles = match the_built_bundles() {
        Ok(bundles) => bundles,
        Err(why) => {
            skipped(test, why);
            return;
        }
    };

    for bundle in bundles {
        let report = the_run_over(bundle);
        let table = report.table();
        eprintln!("{table}");
        for row in &report.rows {
            assert!(
                !row.outcome.is_refusal(),
                "{} was refused by the {} suite: {}\n{table}",
                row.plugin,
                row.suite.name(),
                row.outcome.sentence()
            );
        }
        assert!(
            report.passed(),
            "{} did not pass:\n{table}",
            bundle.display()
        );
        assert_eq!(
            (report.listed, report.described),
            (1, 1),
            "each example is one effect, listed and described:\n{table}"
        );
    }
}
