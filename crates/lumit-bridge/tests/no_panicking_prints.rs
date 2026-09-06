//! No shipping engine code may print with the standard macros.
//!
//! # In plain terms
//!
//! `println!` and `eprintln!` panic when the write fails, and the write fails
//! whenever the console the process started with has gone away — which, for a
//! Windows GUI build, is normal. That panic once reached the user as
//! "Could not measure render times: PanicException(failed printing to stdout:
//! The pipe is being closed)" from a `#[frb(sync)]` call whose only job was to
//! switch the measurements off.
//!
//! Diagnostics go out through the `note!` macro instead (`src/note.rs` here,
//! `src/note.rs` in `lumit-gpu`), which drops a failed write. This test is the
//! gate: it reads the shipping half of every engine source file and fails on a
//! standard print macro, so the fix cannot quietly come undone.
//!
//! Three crates are exempt by design, and all are command-line programs run
//! outside the editor process: `lumit-bench`, whose printed report *is* its
//! output, and the two brokers — `lumit-ofx-broker` and `lumit-aplug-broker`
//! — whose few lines are usage and fatal-error text on their way out. A
//! broker dying is already its designed failure mode (one dry block and a
//! badge), so a print that panics costs nothing a fatal exit did not.

use std::path::{Path, PathBuf};

/// Crates whose console output is the product, not a diagnostic.
const EXEMPT_CRATES: [&str; 3] = ["lumit-bench", "lumit-ofx-broker", "lumit-aplug-broker"];

/// The macros that panic on a failed write.
const BANNED: [&str; 4] = ["println!(", "eprintln!(", "print!(", "eprint!("];

#[test]
fn shipping_engine_code_never_uses_a_panicking_print() {
    let crates = workspace_crates();
    assert!(
        crates.len() > 10,
        "the crates directory was not found where this test expected it"
    );

    let mut hits = Vec::new();
    for file in crates.iter().flat_map(|c| rust_sources(&c.join("src"))) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        // Only the *shipping* half of each file is linted: everything from a
        // `#[cfg(test)]` onward is test code, run from a terminal that is
        // listening, and free to print. This is the same rule the frb-API
        // panic lint in .github/workflows/ci.yml applies.
        for (n, line) in text
            .lines()
            .take_while(|l| !l.starts_with("#[cfg(test)]"))
            .enumerate()
        {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if BANNED.iter().any(|m| code.contains(m)) {
                hits.push(format!("{}:{}: {}", file.display(), n + 1, code));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "a closed console must never panic the editor (docs/14 §4). Use the \
         crate's `note!` macro instead of these:\n{}",
        hits.join("\n")
    );
}

/// Every crate directory in the workspace bar the exempt command-line ones.
fn workspace_crates() -> Vec<PathBuf> {
    let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root.join("crates")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            !EXEMPT_CRATES.contains(&name.as_str())
        })
        .collect()
}

/// Every `.rs` file under a crate's `src`, bar whole-file test modules and the
/// small command-line programs some crates carry in `src/bin`.
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for path in entries.flatten().map(|e| e.path()) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if path.is_dir() {
            if name != "bin" {
                found.extend(rust_sources(&path));
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            found.push(path);
        }
    }
    found
}
