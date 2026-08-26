//! The conformance bench's plugins, fetched and built rather than checked in
//! (docs/impl/ofx-host.md §5 item 1).
//!
//! # In plain terms
//!
//! docs/12 §2.5 names two free plugin sets to prove the OFX host against:
//! **openfx-misc**, Natron's eighty-odd effects, and **ntsc-rs**. Neither can
//! go in this repository — one is a large third-party tree, the other is
//! somebody else's release binary — so this module does what
//! [`media`](crate::media) does for the reference clips: it fetches and builds
//! them into a folder, once, and leaves them alone on every run after that.
//!
//! **Nothing here fails a build.** A machine without `git`, without `cmake`, or
//! without the network answers with a sentence saying which, and the
//! conformance test then runs against Lumit's own test plugin and says the
//! bench was absent. That is the same politeness the media generator has, and
//! it is what keeps a developer who has not built eighty plugins on a green
//! suite.
//!
//! The build recipes are each project's own documented one. They are the part
//! of this module a CI job proves rather than a test: a compiler that is not on
//! this machine cannot be exercised from here.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the bench's bundles live when nothing says otherwise — the same
/// default `lumit-ofx`'s conformance test looks in, so the two need no
/// arrangement beyond this name.
#[must_use]
pub fn bench_dir() -> PathBuf {
    std::env::var_os("LUMIT_OFX_BENCH")
        .and_then(|value| std::env::split_paths(&value).next())
        .unwrap_or_else(|| std::env::temp_dir().join("lumit-ofx-bench"))
}

/// The bundles already in `dir`, by name.
fn bundles_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".ofx.bundle"))
        })
        .collect();
    found.sort();
    found
}

/// Fetch and build the bench into `dir`, or say what stopped it.
///
/// Idempotent: a folder that already holds bundles is handed straight back, so
/// a second run — and a vendored or prebuilt folder somebody dropped in — costs
/// nothing and needs no compiler at all.
///
/// # Errors
///
/// A sentence naming the missing tool, the failed command, or the empty result.
pub fn ensure(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let already = bundles_in(dir);
    if !already.is_empty() {
        return Ok(already);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let work = dir.join("src");
    std::fs::create_dir_all(&work).map_err(|e| format!("creating {}: {e}", work.display()))?;

    // Both are attempted; a failure in one is a line rather than the end, so a
    // machine that can build ntsc-rs but not openfx-misc still gets a bench.
    let mut trouble = Vec::new();
    for build in [openfx_misc, ntsc_rs] {
        if let Err(why) = build(&work, dir) {
            trouble.push(why);
        }
    }

    let found = bundles_in(dir);
    if found.is_empty() {
        return Err(if trouble.is_empty() {
            format!("nothing was built into {}", dir.display())
        } else {
            trouble.join("; ")
        });
    }
    for why in trouble {
        eprintln!("lumit-bench ofx: {why}");
    }
    Ok(found)
}

/// openfx-misc: Natron's plugin set, built with its own CMake project.
fn openfx_misc(work: &Path, into: &Path) -> Result<(), String> {
    // The compiler is asked for **before** the download: a hundred megabytes of
    // somebody else's source, fetched and then found to be unbuildable, is a
    // rude way to say "no cmake here".
    tool("cmake")?;
    let checkout = clone(
        work,
        "openfx-misc",
        "https://github.com/NatronGitHub/openfx-misc.git",
    )?;
    let build = checkout.join("build");
    run(Command::new("cmake")
        .arg("-S")
        .arg(&checkout)
        .arg("-B")
        .arg(&build)
        .arg("-DCMAKE_BUILD_TYPE=Release"))?;
    // **A partial build is still a bench.** openfx-misc is several targets, and
    // one of them (CImg) wants a header its shallow clone does not bring; the
    // other seventy-odd plugins compile perfectly well beside it. So the build's
    // own exit code is a note rather than the end, and what was actually
    // produced is the answer — `collect` says no only when nothing at all was.
    let trouble = run(Command::new("cmake")
        .arg("--build")
        .arg(&build)
        .arg("--config")
        .arg("Release"))
    .err();
    collect(&build, into).map_err(|why| match trouble {
        Some(build) => format!("{why} ({build})"),
        None => why,
    })
}

/// ntsc-rs: one plugin, built by Cargo, laid out as a bundle by hand because
/// its build does not make one.
fn ntsc_rs(work: &Path, into: &Path) -> Result<(), String> {
    tool("cargo")?;
    let checkout = clone(
        work,
        "ntsc-rs",
        "https://github.com/valadaptive/ntsc-rs.git",
    )?;
    run(Command::new("cargo").current_dir(&checkout).args([
        "build",
        "--release",
        "-p",
        "ntsc-rs-openfx-plugin",
    ]))?;

    // Found by extension rather than by name: what Cargo calls the library is
    // the crate's own business and it has been renamed before now.
    let built = std::fs::read_dir(checkout.join("target").join("release"))
        .map_err(|e| format!("reading ntsc-rs's build output: {e}"))?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some(library_extension())
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| stem.contains("openfx"))
        })
        .ok_or_else(|| "ntsc-rs built no plugin library".to_owned())?;

    let contents = into
        .join("NtscRs.ofx.bundle")
        .join("Contents")
        .join(arch_dir());
    std::fs::create_dir_all(&contents).map_err(|e| format!("creating the ntsc-rs bundle: {e}"))?;
    std::fs::copy(&built, contents.join("NtscRs.ofx"))
        .map_err(|e| format!("copying the ntsc-rs plugin: {e}"))?;
    Ok(())
}

/// What a shared library is called at the end on this platform.
const fn library_extension() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

/// The bundle sub-directory this platform's binaries live in. Spelled here
/// rather than borrowed from `lumit-ofx`, because a benchmark harness that
/// nothing depends on should not put an edge into the plugin host to copy one
/// string (docs/05).
const fn arch_dir() -> &'static str {
    if cfg!(target_os = "windows") {
        "Win64"
    } else if cfg!(target_os = "macos") {
        "MacOS"
    } else {
        "Linux-x86-64"
    }
}

/// Clone `url` into `work/name`, or reuse the checkout that is already there.
fn clone(work: &Path, name: &str, url: &str) -> Result<PathBuf, String> {
    let into = work.join(name);
    if into.join(".git").is_dir() {
        return Ok(into);
    }
    tool("git")?;
    run(Command::new("git").args([
        "clone",
        "--depth",
        "1",
        "--recurse-submodules",
        url,
        &into.to_string_lossy(),
    ]))?;
    Ok(into)
}

/// Gather everything under `from` that a host could load into `into`, as
/// bundles.
///
/// Two shapes, because the projects emit two: a finished `*.ofx.bundle`
/// directory (what a Makefile install step leaves), and a **loose `.ofx`
/// binary** (what the CMake build drops in `build/Release`, which is what
/// openfx-misc really produces on Windows). The loose one is wrapped in the
/// bundle layout here — that layout is the host's only way in
/// (docs/impl/ofx-host.md §1), and expecting somebody else's build to have
/// arranged it would be expecting them to know about us.
fn collect(from: &Path, into: &Path) -> Result<(), String> {
    let mut found = 0usize;
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if name.ends_with(".ofx.bundle") {
                    copy_tree(&path, &into.join(&name))?;
                    found += 1;
                } else {
                    stack.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("ofx") {
                let contents = into
                    .join(format!("{name}.bundle"))
                    .join("Contents")
                    .join(arch_dir());
                std::fs::create_dir_all(&contents)
                    .map_err(|e| format!("creating {}: {e}", contents.display()))?;
                std::fs::copy(&path, contents.join(&name))
                    .map_err(|e| format!("copying {}: {e}", path.display()))?;
                found += 1;
            }
        }
    }
    if found == 0 {
        return Err(format!("no plugin was built under {}", from.display()));
    }
    Ok(())
}

/// Copy a directory tree. Small and recursive on purpose: a bundle is a handful
/// of files, and this is the only place the harness needs one.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    let entries =
        std::fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            std::fs::copy(&source, &target)
                .map_err(|e| format!("copying {}: {e}", source.display()))?;
        }
    }
    Ok(())
}

/// Whether a command-line tool answers at all.
fn tool(name: &str) -> Result<(), String> {
    let answered = Command::new(name)
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if answered {
        Ok(())
    } else {
        Err(format!("no {name} on this machine"))
    }
}

/// Run one command, keeping its output for the message when it fails.
fn run(command: &mut Command) -> Result<(), String> {
    let described = format!("{command:?}");
    let output = command
        .output()
        .map_err(|e| format!("{described} did not start: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let tail: String = String::from_utf8_lossy(&output.stderr)
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .join(" / ");
    Err(format!("{described} failed: {tail}"))
}
