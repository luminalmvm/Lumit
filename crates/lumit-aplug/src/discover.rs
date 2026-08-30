//! Finding the audio plugins installed on this machine, and offering them.
//!
//! # In plain terms
//!
//! Everything else in this crate could host a plugin somebody handed it by
//! path. This is the part that goes looking: it reads the standard CLAP folders
//! (and anything `CLAP_PATH` adds), opens each `.clap` file it finds, asks
//! every plugin in it to describe itself, and hands back a list plus a report.
//! After that a plugin is an effect like any other — it goes in the layer's
//! effect stack, its knobs keyframe, and the Graph panel draws it as a node.
//!
//! Three rules the scan follows, all of them docs/12 §2.6's:
//!
//! * **A file that will not load is a line in a report, never a dialogue.**
//!   Somebody else's installer left a broken file on the machine; that is not
//!   worth interrupting the person for, and it must not cost them the other
//!   plugins in the folder.
//! * **The user's switched-off list is consulted before describe**, not after.
//!   A plugin the user has turned off is never described and never
//!   instantiated, so its code never runs.
//! * **Never on the interface's thread.** Opening a module means running other
//!   people's start-up code. The scan is a plain blocking function; the caller
//!   puts it on a worker.
//!
//! **Discovery here loads plugins into this process.** That is AP1's shape and
//! it is not the shipping one: §5 says discovery enumerates *inside the
//! broker*, because a `clap_entry.init` is already third-party code running.
//! AP2 moves it there; the list this answers with does not change shape when it
//! does.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::def::AudioEffectDef;
use crate::describe::describe_module;
use crate::module::Module;
use crate::schema::{schema_of, MATCH_PREFIX};

/// The standard CLAP folders on this platform, plus whatever `CLAP_PATH` adds.
///
/// The Windows two are the ones the standard names — the shared one every
/// installer writes to, and the per-user one that needs no administrator
/// (docs/impl/audio-plugins.md §5).
#[must_use]
pub fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Some(common) = std::env::var_os("COMMONPROGRAMFILES") {
            paths.push(PathBuf::from(common).join("CLAP"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join("Common")
                    .join("CLAP"),
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/CLAP"));
        }
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        paths.push(PathBuf::from("/usr/lib/clap"));
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".clap"));
        }
    }

    if let Some(extra) = std::env::var_os("CLAP_PATH") {
        paths.extend(std::env::split_paths(&extra));
    }
    paths
}

/// The loadable binaries in one directory.
///
/// A `.clap` is a plain shared library on Windows and Linux. On macOS it is a
/// bundle directory whose binary sits at `Contents/MacOS/<name>`, so both
/// shapes are accepted and the directory case is followed one level in.
///
/// Sorted, so two runs discover plugins in the same order — which is what makes
/// an effect list stable between sessions.
#[must_use]
pub fn scan_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("clap") {
            continue;
        }
        if path.is_file() {
            found.push(path);
            continue;
        }
        let inside = path.join("Contents").join("MacOS");
        let Ok(binaries) = std::fs::read_dir(&inside) else {
            continue;
        };
        for binary in binaries.flatten() {
            let binary = binary.path();
            if binary.is_file() {
                found.push(binary);
            }
        }
    }
    found.sort();
    found
}

/// What a scan was asked to do.
#[derive(Debug, Default)]
pub struct ScanOptions {
    /// The directories to look in. [`ScanOptions::standard`] fills this from
    /// [`search_paths`].
    pub paths: Vec<PathBuf>,
    /// Plugin identifiers the user has switched off. Consulted before a plugin
    /// is described, so a disabled plugin's code never runs. The same list the
    /// OFX host reads (K-594).
    pub disabled: BTreeSet<String>,
}

impl ScanOptions {
    /// The standard search paths, nothing disabled — what start-up asks for
    /// before the preferences are read into it.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            paths: search_paths(),
            disabled: BTreeSet::new(),
        }
    }
}

/// One plugin that became an effect.
pub struct DiscoveredPlugin {
    /// The name the catalogue answers to — `clap:` and the plugin's own id.
    pub match_name: String,
    /// The plugin's own identifier, which is what the switched-off list names.
    pub identifier: String,
    /// The name a person sees.
    pub label: String,
    /// Who wrote it.
    pub vendor: String,
    /// Which `.clap` file it came out of.
    pub module: PathBuf,
    /// The definition everything downstream sees (K-593).
    pub def: AudioEffectDef,
}

/// What one scan did.
#[derive(Default)]
pub struct ScanOutcome {
    /// The plugins that can be hosted, in discovery order.
    pub found: Vec<DiscoveredPlugin>,
    /// One calm sentence per file or plugin turned away, in the order it
    /// happened. Nothing here is shown modally.
    pub skipped: Vec<String>,
}

/// Scan the named directories and describe everything hostable in them.
///
/// Blocking, and not to be called from the interface's thread.
#[must_use]
pub fn scan(options: &ScanOptions) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    for dir in &options.paths {
        for binary in scan_dir(dir) {
            scan_module(&binary, options, &mut outcome);
        }
    }
    outcome
}

/// One `.clap` file, opened, described and closed.
fn scan_module(binary: &Path, options: &ScanOptions, outcome: &mut ScanOutcome) {
    let module = match Module::open(binary) {
        Ok(module) => Arc::new(module),
        Err(error) => {
            outcome.skipped.push(skip_line(binary, &error.to_string()));
            return;
        }
    };

    // Before describe, not after: a switched-off plugin is never created, so
    // its code never runs.
    if module
        .entries()
        .iter()
        .all(|entry| options.disabled.contains(&entry.id))
    {
        return;
    }

    let report = describe_module(&module);
    for refused in &report.rejected {
        outcome.skipped.push(skip_line(
            binary,
            &format!("{}: {}", refused.id, refused.reason),
        ));
    }
    for descriptor in &report.described {
        if options.disabled.contains(&descriptor.id) {
            continue;
        }
        let schema = match schema_of(descriptor) {
            Ok(schema) => Box::leak(Box::new(schema)),
            Err(error) => {
                outcome
                    .skipped
                    .push(skip_line(binary, &format!("{}: {error}", descriptor.id)));
                continue;
            }
        };
        outcome.found.push(DiscoveredPlugin {
            match_name: format!("{MATCH_PREFIX}{}", descriptor.id),
            identifier: descriptor.id.clone(),
            label: descriptor.label.clone(),
            vendor: descriptor.vendor.clone(),
            module: binary.to_path_buf(),
            def: AudioEffectDef::new(descriptor, schema, binary),
        });
    }
}

/// One line of the report, in the shape the OFX scan's are.
fn skip_line(binary: &Path, reason: &str) -> String {
    format!("{}: {reason}", binary.display())
}
