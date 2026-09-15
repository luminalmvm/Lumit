//! Loading ONNX Runtime, once, from the folder the user installed it into
//! (docs/impl/addons.md §3).
//!
//! Nothing here is fetched, probed on a timer or loaded at start-up. The
//! library is opened the first time something asks for it and never again,
//! and every failure comes back as a sentence rather than a fault.
//!
//! # Thread role and contract
//!
//! [`load`] runs on whichever thread asked first and every later caller waits
//! behind the same `OnceLock`, so the library is opened exactly once per
//! process (14-ENGINEERING-RULES §1.3). It holds no lock across the load
//! itself, and nothing in Lumit builds a session before it has returned `Ok`:
//! `ort`'s own lazy path looks for the library beside the executable and
//! panics when it is not there (§13).

use std::io::Write;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{error::MlError, store};

/// The addon id the runtime is always installed under. Exactly one may be.
pub const RUNTIME_ID: &str = "runtime";

/// The shared library this platform's runtime addon carries.
pub const LIBRARY: &str = if cfg!(windows) {
    "onnxruntime.dll"
} else if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else {
    "libonnxruntime.so"
};

/// The execution provider a session asks for first here. ONNX Runtime falls
/// back to the CPU on its own, and which one actually took a session is read
/// back and recorded (§7).
pub const PROVIDER: &str = if cfg!(windows) {
    "DirectML"
} else if cfg!(target_os = "macos") {
    "CoreML"
} else {
    "CPU"
};

/// What the CPU provider is called wherever a session lands on it.
pub const CPU: &str = "CPU";

/// Which provider a run made now is made by: the one a session really took,
/// once one has been opened, and the one this platform asks for first before
/// that.
///
/// This is what every key a model result is filed under carries (§7), so a
/// machine whose accelerator will not register files its answers under `CPU`
/// rather than under a name nothing on it can produce. The answer can move
/// once, the first time a session is built; it is a cache miss and the truth
/// afterwards.
#[must_use]
pub fn provider() -> &'static str {
    TAKEN.get().copied().unwrap_or(PROVIDER)
}

/// Say which provider a session actually took. Called by [`crate::session`]
/// for a session that asked for the platform's own, and by nothing else: a
/// session built deliberately on the processor says nothing about what this
/// machine can do.
pub(crate) fn took(provider: &'static str) {
    let _ = TAKEN.set(provider);
}

/// The first answer [`took`] was given.
static TAKEN: OnceLock<&'static str> = OnceLock::new();

/// The environment variable that turns "no model runtime" from a skip into a
/// failure. Set it on any machine that is *supposed* to have one.
pub const REQUIRE_RUNTIME_ENV: &str = "LUMIT_REQUIRE_ML";

/// The loaded library, as the badge reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The provider a session will try first on this platform.
    pub provider: &'static str,
    /// The version the library itself reports.
    pub version: String,
}

/// What the Addons page says about the runtime row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// Nothing is installed.
    Missing,
    /// The folder is there and nothing has asked for it yet.
    Present,
    /// [`load`] returned `Ok` since Lumit started.
    Loaded {
        /// Which provider a session tries first.
        provider: String,
        /// What the library reports itself as.
        version: String,
    },
    /// [`load`] was asked and refused. Carries the library's own sentence.
    Failed {
        /// Untranslated, and marked as the library's own words where it is
        /// shown.
        detail: String,
    },
}

/// The one outcome this process has. A failure is remembered as well as a
/// success: `ort` keeps its own handle behind a `OnceLock` too, so a second
/// attempt in the same process would answer the same thing.
static LOADED: OnceLock<Result<Loaded, String>> = OnceLock::new();

/// Open the runtime in `dir` and keep it for the life of the process.
///
/// # Errors
///
/// [`MlError::RuntimeMissing`] when the folder holds no library, which is not
/// recorded so that installing one and asking again works, and
/// [`MlError::RuntimeFailed`] with the library's own words when it will not
/// load.
pub fn load(dir: &Path) -> Result<&'static Loaded, MlError> {
    let library = dir.join(LIBRARY);
    if !library.is_file() {
        return Err(MlError::RuntimeMissing);
    }
    match LOADED.get_or_init(|| open(&library)) {
        Ok(loaded) => Ok(loaded),
        Err(detail) => Err(MlError::RuntimeFailed(detail.clone())),
    }
}

/// [`load`], pointed at the runtime addon the user installed.
///
/// # Errors
///
/// [`MlError::RuntimeMissing`] when no runtime addon is installed, and
/// whatever [`load`] answers otherwise.
pub fn load_installed() -> Result<&'static Loaded, MlError> {
    let installed = store::pack(RUNTIME_ID).ok_or(MlError::RuntimeMissing)?;
    load(&installed.path)
}

/// Where the runtime addon is, if it is installed.
#[must_use]
pub fn dir() -> Option<PathBuf> {
    store::pack(RUNTIME_ID).map(|installed| installed.path)
}

/// Whether the library is open in this process.
///
/// Asked before anything builds a session, because `ort`'s own lazy path goes
/// looking for the library beside the executable and `expect`s when it is not
/// there (§13), and asked before an install or a remove touches the runtime's
/// folder, because an open library cannot be deleted out from under itself.
#[must_use]
pub fn loaded() -> bool {
    matches!(LOADED.get(), Some(Ok(_)))
}

/// The runtime row's reading. Read from the store until something has asked
/// for the library, and from the load's own outcome after that.
#[must_use]
pub fn status() -> RuntimeStatus {
    if let Some(outcome) = LOADED.get() {
        return match outcome {
            Ok(loaded) => RuntimeStatus::Loaded {
                provider: loaded.provider.to_owned(),
                version: loaded.version.clone(),
            },
            Err(detail) => RuntimeStatus::Failed {
                detail: detail.clone(),
            },
        };
    }
    match store::pack(RUNTIME_ID) {
        Some(installed) if !installed.broken => RuntimeStatus::Present,
        _ => RuntimeStatus::Missing,
    }
}

/// The load itself, run once.
fn open(library: &Path) -> Result<Loaded, String> {
    #[cfg(windows)]
    if let Some(folder) = library.parent() {
        put_on_path(folder);
    }

    let environment = ort::init_from(library).map_err(|e| e.to_string())?;
    // False means an environment was already configured, which is not a
    // failure: the library is loaded either way.
    let _ = environment.commit();

    Ok(Loaded {
        provider: PROVIDER,
        version: version(),
    })
}

/// Put `folder` at the front of the process `PATH`.
///
/// Windows resolves a loaded library's own dependents by the standard search
/// order, which starts at the executable's folder and ends at `PATH`, not
/// beside the library that wants them. Without this `onnxruntime.dll` loads
/// and `DirectML.dll` sitting right next to it does not (§13).
#[cfg(windows)]
fn put_on_path(folder: &Path) {
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut folders = vec![folder.to_path_buf()];
    folders.extend(std::env::split_paths(&existing));
    if let Ok(joined) = std::env::join_paths(folders) {
        std::env::set_var("PATH", joined);
    }
}

/// What the row says the runtime is.
///
/// The version the runtime addon's own manifest pins, which is the catalogue's
/// pin and the thing that changes when the user installs a different one. Not
/// the library's, because there is no way to ask it: ONNX Runtime reports a
/// version through `OrtGetApiBase`, `ort` reads it on the way in and does not
/// hand it back, and the DirectML build's build-info string carries a commit
/// rather than a release number. With nothing installed - which only happens
/// when something pointed [`load`] at a folder of its own - the library's own
/// words about its build are what is left.
fn version() -> String {
    store::pack(RUNTIME_ID).map_or_else(
        || ort::info().to_owned(),
        |installed| installed.manifest.version,
    )
}

/// What a test that needs the runtime does when there is none.
///
/// The pure half of this crate never skips; the half that opens a library
/// does, on every machine that has not installed one, and CI is one of those.
/// A skip and a pass look identical in a summary, so `LUMIT_REQUIRE_ML` is
/// what tells them apart on a machine that is supposed to have the runtime.
///
/// Call it at the skip site and return:
/// ```ignore
/// let Some(dir) = std::env::var_os("LUMIT_ML_RUNTIME_DIR").map(PathBuf::from) else {
///     lumit_ml::no_runtime();
///     return;
/// };
/// ```
pub fn no_runtime() {
    let set = std::env::var(REQUIRE_RUNTIME_ENV).ok();
    assert!(
        !runtime_is_required(set.as_deref()),
        "no model runtime, but {REQUIRE_RUNTIME_ENV} is set - this machine is \
         supposed to have one (point LUMIT_ML_RUNTIME_DIR at the folder \
         holding {LIBRARY}, or unset the variable to skip)"
    );
    // A closed console must never panic the editor (docs/14), so the skip is
    // written and the result dropped rather than printed.
    let _ = writeln!(std::io::stderr(), "skipping: no model runtime");
}

/// Whether [`REQUIRE_RUNTIME_ENV`]'s value demands a runtime. Unset, empty
/// and `0` all mean "skip politely"; anything else means "this machine has
/// one, and not finding it is the bug". Split out from [`no_runtime`] so the
/// rule can be tested without a process-global environment variable in a
/// parallel suite.
#[must_use]
fn runtime_is_required(value: Option<&str>) -> bool {
    matches!(value, Some(v) if !v.is_empty() && v != "0")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **A machine that is supposed to have the runtime must fail, not skip.**
    /// Every test here that opens a library skips itself without one, which is
    /// how a green job could prove nothing at all: a skip and a pass look
    /// identical in the summary. `LUMIT_REQUIRE_ML` is what tells the
    /// difference, so the rule it encodes is pinned here rather than only in
    /// the build's own configuration.
    #[test]
    fn requiring_the_runtime_is_opt_in_and_zero_still_means_skip() {
        assert!(!runtime_is_required(None), "a laptop keeps the polite skip");
        assert!(!runtime_is_required(Some("")), "an empty value is unset");
        assert!(!runtime_is_required(Some("0")), "0 turns it off explicitly");
        assert!(runtime_is_required(Some("1")), "a machine with one sets 1");
        assert!(runtime_is_required(Some("yes")));
    }

    /// **A folder with no library is Missing, not Failed.** The two say
    /// different things to the user: one is "install the runtime", the other
    /// is "the runtime you installed will not open", and a wrong answer sends
    /// them to the wrong button.
    #[test]
    fn a_folder_without_the_library_is_missing() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(load(empty.path()).unwrap_err(), MlError::RuntimeMissing);
    }

    /// **On a machine with the runtime, it loads and says what it is.** Run
    /// for real by pointing `LUMIT_ML_RUNTIME_DIR` at the folder holding the
    /// library; skipped politely everywhere else, which is every CI runner.
    #[test]
    fn the_runtime_loads_and_reports_its_provider_and_version() {
        let Some(dir) = crate::test_support::runtime_dir() else {
            no_runtime();
            return;
        };
        let loaded = load(&dir).expect("the runtime at LUMIT_ML_RUNTIME_DIR would not load");
        assert_eq!(loaded.provider, PROVIDER);
        assert!(
            !loaded.version.is_empty(),
            "the row would have nothing to say"
        );
        assert_eq!(
            status(),
            RuntimeStatus::Loaded {
                provider: PROVIDER.to_owned(),
                version: loaded.version.clone(),
            }
        );
    }
}
