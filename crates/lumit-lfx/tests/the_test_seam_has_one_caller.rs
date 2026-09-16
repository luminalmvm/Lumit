//! The one seam another crate's suite reaches the session tables through has
//! one caller, and this is what says so.
//!
//! # In plain terms
//!
//! `lumit_lfx::discover::for_test::file_refusal` writes straight into the
//! session `REFUSED` table that `refusal_of` reads and `badge_of` badges from.
//! It exists because `lumit-bridge`'s suite has to prove that `badge_of` asks
//! that table before it falls through to the namespace arm, and a real scan
//! needs a bundle, a broker executable and a second process. It is compiled
//! into the shipping library like anything else - a feature would not hold it,
//! since Cargo unifies a dev-dependency's features with the ordinary one in
//! the same build - so anything in the tree could file a refusal against an
//! installed, working plugin and have its layers badge `plugin_refused` with
//! words nobody scanned for.
//!
//! What keeps that from happening is the rule rather than the spelling: one
//! caller, named here. A second one fails this test, which is the conversation
//! the seam is worth having.
//!
//! Only each crate's `src` is read, which is where shipping code and its inline
//! suites live - and is why this file, which names the seam a dozen times, is
//! not looking at itself.
//!
//! The name is swept rather than the whole path, so an import that shortens it
//! is caught too. `discover.rs` spells it a second time for the scan's own
//! private helper, which is the same word for the same thing and inside the one
//! file that may say it: the rule is that **no other file names it at all**.

use std::path::{Path, PathBuf};

/// The seam, by the name a call site spells it.
const SEAM: &str = "file_refusal";

/// Where it may be named, from the crate directory down. Its own definition,
/// and the one case it was written for.
const CALLERS: [&str; 2] = ["lumit-lfx/src/discover.rs", "lumit-bridge/src/api/tests.rs"];

#[test]
fn the_refusal_seam_is_named_in_two_files_and_no_others() {
    let crates = workspace_crates();
    assert!(
        crates.len() > 10,
        "the crates directory was not found where this test expected it"
    );

    let mut found: Vec<String> = Vec::new();
    for file in crates
        .iter()
        .flat_map(|krate| rust_sources(&krate.join("src")))
    {
        if is_a_caller(&file) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if line.contains(SEAM) {
                found.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }

    assert!(
        found.is_empty(),
        "{SEAM} files a refusal against a plugin nothing ever scanned, and has \
         one caller by design (docs/impl/lfx.md §4.3). These are not it:\n{}",
        found.join("\n")
    );
}

/// Whether this path is one of the [`CALLERS`].
fn is_a_caller(file: &Path) -> bool {
    let path = file.to_string_lossy().replace('\\', "/");
    CALLERS.iter().any(|tail| path.ends_with(tail))
}

/// Every crate directory in the workspace.
fn workspace_crates() -> Vec<PathBuf> {
    let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root.join("crates")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// Every `.rs` file under a crate's `src`.
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            found.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            found.push(path);
        }
    }
    found
}
