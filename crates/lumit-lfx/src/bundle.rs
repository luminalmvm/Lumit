//! Where an LFX plugin lives on disk, and how a scan finds it
//! (docs/impl/lfx.md §5.1, §5.2).
//!
//! # In plain terms
//!
//! An LFX plugin ships as a folder called something `.lfx.bundle`, with a
//! listing and one architecture directory per build inside it:
//!
//! ```text
//! Example.lfx.bundle/
//!   Contents/
//!     lfx.toml
//!     linux-x86_64/Example.lfx
//!     win-x86_64/Example.lfx
//! ```
//!
//! This module answers the two questions that asks. Which of those directories
//! belongs to the machine Lumit is running on - [`payload`] - and which folders
//! are looked in at all - [`search_paths`]. Neither opens anything: nothing
//! here reads the listing, let alone the payload. The listing is the broker's
//! to parse (§11 item 7) and the payload is the broker's to open, and this
//! module hands the supervisor two paths and stops.
//!
//! # An ordered list rather than one string
//!
//! The OFX host has a single `BUNDLE_ARCH_DIR` per platform, which is the
//! standard's own answer and has no arm64 spelling for Windows or Linux at all.
//! LFX declares an **ordered list** instead and tries it in order, so a bundle
//! that ships a universal macOS binary and a bundle that ships two per-CPU ones
//! both work, and a machine that gains a second architecture does not need the
//! vendor to re-issue.
//!
//! The list is **per target, not per platform**: the operating system alone
//! does not say which binaries a process can load, and an Intel Mac handed a
//! list that named `macos-arm64` first would take the wrong dylib out of the
//! very bundle the list exists for. [`arch_dirs`] names this build's own first
//! and a foreign CPU only where the platform emulates one.
//!
//! [`payload`] follows `lumit_aplug::vst3::payload`'s shape, ceiling and
//! sorted fallback, with one deliberate difference: LFX owns all seven
//! architecture names ([`ALL_ARCH_DIRS`]), so the fallback passes over the six
//! that are not this build's rather than treating a known foreign build as an
//! unfamiliar one.

use std::path::{Path, PathBuf};

/// What a bundle directory's name ends with.
pub const BUNDLE_SUFFIX: &str = ".lfx.bundle";

/// The directory inside a bundle that everything lives in - the same constant
/// [`crate::manifest`] reads the listing out of, re-exported here so a caller
/// building a path has one place to look.
pub use crate::manifest::CONTENTS_DIR;

/// The extension the payload carries.
pub const PAYLOAD_EXTENSION: &str = "lfx";

/// The environment variable a person points at plugins kept somewhere else
/// with, split with `std::env::split_paths` and **appended, never replacing**.
pub const PLUGIN_PATH_ENV: &str = "LFX_PLUGIN_PATH";

/// How far below a search path a bundle is looked for.
///
/// The OFX walk's number and the OFX walk's reasons: vendors install into a
/// suite folder of their own rather than at the top of the plugin directory, so
/// a scan that read one level found none of them. Four levels is more than any
/// installer uses, and is also the floor under a folder that contains itself -
/// this walk follows directories, and a symbolic link back up one would
/// otherwise never end.
pub const MAX_BUNDLE_DEPTH: usize = 4;

/// Every architecture directory LFX names, on every platform.
///
/// The closed vocabulary, which is the thing LFX has and the standards it sits
/// beside have not: these seven spellings are this project's own, so a folder
/// called `win-x86_64` on a Linux machine is a **known foreign** build rather
/// than an unfamiliar one, and [`payload`] can say so instead of handing the
/// loader a PE file and reporting whatever `dlopen` makes of it.
pub const ALL_ARCH_DIRS: &[&str] = &[
    "win-x86_64",
    "win-arm64",
    "macos-universal",
    "macos-arm64",
    "macos-x86_64",
    "linux-x86_64",
    "linux-aarch64",
];

/// The architecture directories **this build** can load, most likely first
/// (docs/impl/lfx.md §5.1).
///
/// One list per target rather than per platform, because the operating system
/// alone does not say which binaries a process can load. An Intel Mac reading
/// a list that named `macos-arm64` before `macos-x86_64` would hand the broker
/// the wrong dylib out of a bundle that shipped both - and a bundle shipping
/// both is the shape this list exists to serve.
///
/// So each target names its own build first, and a foreign CPU appears only
/// where the platform actually emulates one: macOS names `macos-x86_64` last on
/// Apple silicon, because Rosetta will run it and a vendor who has not rebuilt
/// is better served slowly than not at all. Windows on arm64 names
/// `win-x86_64` for the same reason. Nothing else lists a CPU it cannot run.
///
/// *ponytail:* a **target** this build does not name at all gets an empty list
/// and falls through to [`payload`]'s sorted walk, which is the honest answer:
/// LFX has no spelling for that machine yet, so the only directory that could
/// serve it is one this vocabulary has never heard of. That is true on both
/// axes and the operating system is the easier one to get wrong - the six
/// spellings below name three operating systems by name, so FreeBSD, illumos,
/// Android and iOS get the empty list rather than Linux's. A list chosen by
/// "not Windows and not macOS" would hand a FreeBSD box `linux-x86_64`, and
/// [`payload`] would spawn a broker on an ELF built for another operating
/// system and report "the module would not load" where the truth - and §7.3's
/// Installed row - is that this bundle carries no build for this machine. It is
/// the same mistake the per-target list exists to stop on the CPU axis.
#[must_use]
pub fn arch_dirs() -> &'static [&'static str] {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        &["win-x86_64"]
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        &["win-arm64", "win-x86_64"]
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        &["macos-universal", "macos-x86_64"]
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        &["macos-universal", "macos-arm64", "macos-x86_64"]
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        &["linux-x86_64"]
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        &["linux-aarch64"]
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        &[]
    }
}

/// The payload inside a bundle: the first `.lfx` in `Contents/<arch>/`, by
/// name.
///
/// The named directories are tried in [`arch_dirs`]' order, and only then does
/// a directory **no** spelling in [`ALL_ARCH_DIRS`] names get a look - sorted,
/// so two runs pick the same file.
///
/// **The file's own name is not compared with the bundle's.** §5.1 writes the
/// layout `<arch>/<Name>.lfx` because that is what an installer should produce,
/// and this reads the first `.lfx` in the directory whatever it is called,
/// which is `lumit_aplug::vst3::payload`'s rule and is what makes a bundle
/// renamed on disk - the ordinary outcome of a person tidying a folder - go on
/// loading. A directory holding two `.lfx` files resolves to whichever sorts
/// first; stably, which is the property that matters here, and the extension
/// is the only thing that says what a payload is.
///
/// The fallback skips every one of the seven on purpose, which is where this
/// parts company with `lumit_aplug::vst3::payload`. That host knows only its
/// own platform's spellings of somebody else's standard, so a folder it cannot
/// name might be one it should have taken. LFX owns all seven names, so a
/// `win-x86_64` on a Linux machine is a build for another CPU and not a guess
/// worth making: taking it would spawn a broker on a PE file and report "the
/// module would not load" where the truth is that this bundle carries no build
/// for this machine - which is the sentence §7.3's Installed row prints.
///
/// *ponytail:* a bundle shipping two architectures neither of whose folders
/// this vocabulary names is still a coin toss, which is why the named ones are
/// tried first and why the fallback admits `.lfx` files only. A bundle whose
/// payload is called something else is a bundle this host cannot open, and
/// finding that out at the loader is the honest place for it.
#[must_use]
pub fn payload(bundle: &Path) -> Option<PathBuf> {
    let contents = bundle.join(CONTENTS_DIR);
    for arch in arch_dirs() {
        if let Some(found) = first_payload(&contents.join(arch)) {
            return Some(found);
        }
    }
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&contents)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && !is_another_machines(path))
        .collect();
    folders.sort();
    folders.iter().find_map(|dir| first_payload(dir))
}

/// Whether a directory is one of the seven LFX names that is not this build's -
/// a build for another CPU, which is a thing to pass over rather than a thing
/// to guess at.
fn is_another_machines(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| ALL_ARCH_DIRS.contains(&name))
}

/// The first `.lfx` in a directory, by name. `None` for a directory that is not
/// there, which is the ordinary answer for an architecture this bundle does not
/// ship.
fn first_payload(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|ext| ext.to_str()) == Some(PAYLOAD_EXTENSION)
        })
        .collect();
    files.sort();
    files.into_iter().next()
}

/// The directories LFX bundles live in: the platform's own, plus whatever
/// [`PLUGIN_PATH_ENV`] adds, plus Lumit's own addons folder
/// (docs/impl/lfx.md §5.2).
///
/// **Appended, never replacing**, in both cases. A person who keeps plugins
/// somewhere else is adding a folder, not taking the standard ones away, and a
/// variable that replaced them would make one line in a launcher script hide
/// every plugin on the machine.
///
/// The addons folder is last and matters most on Linux: inside a Flatpak `/usr`
/// is the runtime's own, so the standard location is empty however correct it
/// is, and [`lumit_ipc::addons_dir`] redirects into a writable place. §7.3's
/// read-only search-path rows exist so a person can see that rather than deduce
/// it.
#[must_use]
pub fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(target_os = "windows")]
    paths.push(PathBuf::from(r"C:\Program Files\Common Files\LFX\Plugins"));
    #[cfg(target_os = "macos")]
    paths.push(PathBuf::from("/Library/LFX/Plugins"));
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    paths.push(PathBuf::from("/usr/lib/lfx"));

    if let Some(extra) = std::env::var_os(PLUGIN_PATH_ENV) {
        paths.extend(std::env::split_paths(&extra));
    }
    paths.extend(lumit_ipc::addons_dir());
    paths
}

/// The bundle directories at or below one directory: `**/*.lfx.bundle`.
///
/// **Bundles rather than payloads.** The bundle is what a person points at,
/// what the listing lives in, and what one broker process is kept per; which
/// binary inside it belongs to this machine is [`payload`]'s question and is
/// asked once, where the broker is spawned.
///
/// Sorted, so two runs discover in the same order - which is what makes an
/// effect list stable between sessions - and never descending into a bundle
/// looking for another, because what is inside one is that plugin's own
/// business.
///
/// *ponytail:* the walk follows dot-prefixed directories like any other, which
/// is deliberate for a person who keeps plugins under `~/.local` and is the
/// reason §6.2's unpack stages **outside** the search path rather than in a
/// `.staging` under it (§11 item 17). A half-written bundle in a searched
/// folder is discoverable while it is half-written.
#[must_use]
pub fn scan_dir(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_bundles(dir, 0, &mut found);
    found.sort();
    found
}

/// One directory's worth of the walk: the bundles in it, then the folders under
/// it, until [`MAX_BUNDLE_DEPTH`].
fn collect_bundles(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(BUNDLE_SUFFIX))
        {
            found.push(path);
            continue;
        }
        if depth < MAX_BUNDLE_DEPTH {
            collect_bundles(&path, depth + 1, found);
        }
    }
}

/// Every bundle in every search path.
#[must_use]
pub fn discover() -> Vec<PathBuf> {
    search_paths()
        .iter()
        .flat_map(|dir| scan_dir(dir))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use super::*;

    /// `LFX_PLUGIN_PATH` is read out of the whole process's environment, so the
    /// case that sets it takes a lock rather than racing another that reads it.
    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lay out a bundle with a payload in one architecture directory.
    fn a_bundle_at(root: &Path, name: &str, arch: &str) -> PathBuf {
        let bundle = root.join(format!("{name}{BUNDLE_SUFFIX}"));
        let dir = bundle.join(CONTENTS_DIR).join(arch);
        std::fs::create_dir_all(&dir).expect("the bundle directory");
        std::fs::write(dir.join(format!("{name}.{PAYLOAD_EXTENSION}")), b"payload")
            .expect("the payload");
        bundle
    }

    /// §5.2: the platform's own folder, then whatever the variable adds, then
    /// Lumit's own addons directory - **appended, never replacing**, which is
    /// the rule a test already pins for both older hosts.
    #[test]
    fn the_search_paths_are_the_standard_ones_plus_the_variable_and_the_addons_folder() {
        let test = "the_search_paths_are_the_standard_ones_plus_the_variable_and_the_addons_folder";
        let _guard = env_lock();
        std::env::remove_var(PLUGIN_PATH_ENV);
        let standard = search_paths();
        assert!(
            !standard.is_empty(),
            "this platform has a standard location"
        );
        // A bare container or a service account has no home directory, so
        // there is no addons folder to append - the same skip the other two
        // hosts take in their own `the_addons_folder_is_one_of_the_folders_a_
        // scan_looks_in`. Everything below this is about the variable and runs
        // either way.
        match lumit_ipc::addons_dir() {
            Some(addons) => assert_eq!(
                standard.last().map(PathBuf::as_path),
                Some(addons.as_path()),
                "the addons folder is searched, and is last"
            ),
            None => eprintln!("{test}: this machine has no data directory to put addons in"),
        }

        // Not a drive letter: `split_paths` splits on `:` on Unix, so
        // "Z:/elsewhere" would arrive as two paths there.
        let extra = std::env::temp_dir().join("lumit-lfx-elsewhere");
        std::env::set_var(PLUGIN_PATH_ENV, &extra);
        let widened = search_paths();
        std::env::remove_var(PLUGIN_PATH_ENV);

        assert_eq!(
            widened.len(),
            standard.len() + 1,
            "{PLUGIN_PATH_ENV} adds to the standard folders, it never replaces them"
        );
        assert!(widened.contains(&extra), "and the folder it named is there");
        assert!(
            widened.starts_with(&standard[..1]),
            "the standard location is still first: {widened:?}"
        );
    }

    /// The walk reaches a vendor's suite folder, stops four levels down, never
    /// looks inside a bundle for another, and answers in the same order twice -
    /// which is what makes an effect list stable between sessions (§5.1).
    #[test]
    fn a_bundle_is_found_at_any_depth_and_never_inside_another() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let arch = arch_dirs()[0];

        let top = a_bundle_at(root.path(), "Top", arch);
        let suite = root.path().join("Vendor").join("Suite");
        std::fs::create_dir_all(&suite).expect("the suite folder");
        let nested = a_bundle_at(&suite, "Nested", arch);

        // Exactly at the ceiling, and one past it.
        let deep = root.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(&deep).expect("the deep folder");
        let just_reachable = a_bundle_at(&deep, "Reachable", arch);
        let too_deep = deep.join("e");
        std::fs::create_dir_all(&too_deep).expect("the deeper folder");
        let unreachable = a_bundle_at(&too_deep, "Unreachable", arch);

        // A bundle inside a bundle is that plugin's own business.
        let hidden = a_bundle_at(&top.join(CONTENTS_DIR), "Hidden", arch);

        let found = scan_dir(root.path());
        assert!(found.contains(&top), "the bundle at the top: {found:?}");
        assert!(found.contains(&nested), "the vendor's suite: {found:?}");
        assert!(
            found.contains(&just_reachable),
            "four levels down is still searched: {found:?}"
        );
        assert!(
            !found.contains(&unreachable),
            "five levels down is past the ceiling: {found:?}"
        );
        assert!(
            !found.contains(&hidden),
            "a bundle is never opened looking for another: {found:?}"
        );

        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted, "the walk is sorted");
        assert_eq!(found, scan_dir(root.path()), "and stable between runs");
    }

    /// §5.1's ordered per-target list: the architectures are tried in order,
    /// and the first one this build names wins.
    ///
    /// Laid down over **all seven** of [`ALL_ARCH_DIRS`] rather than over this
    /// build's own list, because three of the six targets name exactly one
    /// directory - Linux x86-64 among them, which is the machine CI runs - and
    /// a case that built its fixture from `arch_dirs()` would assert an
    /// ordering over a list of one and pass without comparing anything. A
    /// bundle holding every name LFX has is an ordering question on every
    /// target. The per-CPU pair is
    /// `a_bundle_shipping_both_builds_hands_over_this_machines`'s.
    #[test]
    fn the_payload_is_the_first_architecture_this_platform_ships() {
        let test = "the_payload_is_the_first_architecture_this_platform_ships";
        let archs = arch_dirs();
        let Some(preferred) = archs.first().copied() else {
            eprintln!("{test}: skipped - LFX names no directory for this target yet");
            return;
        };

        let root = tempfile::tempdir().expect("a temporary directory");
        let bundle = root.path().join(format!("Every{BUNDLE_SUFFIX}"));
        for arch in ALL_ARCH_DIRS {
            let dir = bundle.join(CONTENTS_DIR).join(arch);
            std::fs::create_dir_all(&dir).expect("the architecture directory");
            std::fs::write(dir.join(format!("Every.{PAYLOAD_EXTENSION}")), b"payload")
                .expect("the payload");
        }
        let found = payload(&bundle).expect("a build for this machine");
        assert_eq!(
            found.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new(preferred)),
            "the first architecture this build names, out of all seven: {found:?}"
        );

        // And the last name on this build's list is still reached when it is
        // the only one there, which is what makes the list ordered rather than
        // a single preference with a fallback.
        let last = archs[archs.len() - 1];
        let one = a_bundle_at(root.path(), "One", last);
        assert_eq!(
            payload(&one),
            Some(
                one.join(CONTENTS_DIR)
                    .join(last)
                    .join(format!("One.{PAYLOAD_EXTENSION}"))
            ),
            "the only architecture it ships"
        );
    }

    /// The two per-CPU directories a vendor on this operating system ships,
    /// x86-64 first - the pair §5.1's ordered list exists to choose between.
    ///
    /// `None` on an operating system LFX has no spelling for. Named rather than
    /// reached by `else`, on the same reasoning [`arch_dirs`] names three: a
    /// FreeBSD box is not a Linux one, and a model that said it was would let
    /// the list under test say so too.
    fn per_cpu_dirs() -> Option<(&'static str, &'static str)> {
        match std::env::consts::OS {
            "windows" => Some(("win-x86_64", "win-arm64")),
            "macos" => Some(("macos-x86_64", "macos-arm64")),
            "linux" => Some(("linux-x86_64", "linux-aarch64")),
            _ => None,
        }
    }

    /// The directory this machine's own build goes in, read off the operating
    /// system and the CPU the test is running on rather than off the list under
    /// test. `None` on a target LFX has no spelling for.
    fn this_machines_dir() -> Option<&'static str> {
        let (intel, arm) = per_cpu_dirs()?;
        match std::env::consts::ARCH {
            "x86_64" => Some(intel),
            "aarch64" => Some(arm),
            _ => None,
        }
    }

    /// Every directory a binary **this process can load** could legitimately
    /// sit in: this machine's own CPU, a universal build where the platform has
    /// one, and the single foreign CPU the platform emulates.
    ///
    /// Worked out from `std::env::consts` - the machine the test is running on -
    /// rather than from [`arch_dirs`], so that a list chosen by operating
    /// system alone fails here instead of deciding for itself what counts as
    /// foreign.
    fn runnable_here() -> Vec<&'static str> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => vec!["win-x86_64"],
            ("windows", "aarch64") => vec!["win-arm64", "win-x86_64"],
            ("macos", "x86_64") => vec!["macos-universal", "macos-x86_64"],
            ("macos", "aarch64") => vec!["macos-universal", "macos-arm64", "macos-x86_64"],
            ("linux", "x86_64") => vec!["linux-x86_64"],
            ("linux", "aarch64") => vec!["linux-aarch64"],
            _ => Vec::new(),
        }
    }

    /// §5.1's whole reason for being: a vendor ships **both** per-CPU builds in
    /// one bundle, and the one that comes back is the one this process can
    /// load. A list chosen by operating system alone would answer for the other
    /// CPU on half the machines it was written for, and the broker would be
    /// spawned on a binary the loader refuses.
    #[test]
    fn a_bundle_shipping_both_builds_hands_over_this_machines() {
        let test = "a_bundle_shipping_both_builds_hands_over_this_machines";
        let (Some(mine), Some((intel, arm))) = (this_machines_dir(), per_cpu_dirs()) else {
            eprintln!("{test}: skipped - LFX names no directory for this target yet");
            return;
        };

        // The list itself, before any bundle: every directory it names is one
        // this process can load. A list chosen by `target_os` alone names
        // `macos-arm64` on an Intel Mac and `linux-aarch64` on an x86-64 Linux
        // box, and fails here.
        let runnable = runnable_here();
        for named in arch_dirs() {
            assert!(
                runnable.contains(named),
                "{named} is not a build this machine can load: {:?}",
                arch_dirs()
            );
        }

        let root = tempfile::tempdir().expect("a temporary directory");
        let bundle = a_bundle_at(root.path(), "Both", intel);
        let arm_dir = bundle.join(CONTENTS_DIR).join(arm);
        std::fs::create_dir_all(&arm_dir).expect("the second architecture");
        std::fs::write(
            arm_dir.join(format!("Both.{PAYLOAD_EXTENSION}")),
            b"payload",
        )
        .expect("the second payload");

        let found = payload(&bundle).expect("one of the two builds");
        assert_eq!(
            found.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new(mine)),
            "the build this machine can load, not the first one the list names \
             for the platform: {found:?}"
        );
        assert_eq!(
            found.file_name(),
            Some(std::ffi::OsStr::new(&format!("Both.{PAYLOAD_EXTENSION}"))),
            "and it is the payload inside it"
        );
    }

    /// A bundle whose only build is for another CPU has **no** payload here -
    /// it is not a folder to guess at. LFX owns all seven architecture names,
    /// so `win-x86_64` on a Linux machine is a known foreign build, and taking
    /// it would report "the module would not load" where the truth is that this
    /// bundle carries no build for this machine (§14 item 4).
    #[test]
    fn a_bundle_with_only_another_platforms_build_has_no_payload_here() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let runnable = runnable_here();
        for foreign in ALL_ARCH_DIRS
            .iter()
            .filter(|name| !runnable.contains(*name))
        {
            let bundle = a_bundle_at(root.path(), &format!("For-{foreign}"), foreign);
            assert_eq!(
                payload(&bundle),
                None,
                "{foreign} is a build for another machine, not a fallback"
            );
        }
    }

    /// A folder this build cannot name is a coin toss the named list is tried
    /// first to avoid - and when it is all there is, only a `.lfx` in it will
    /// do. A bundle whose payload is called something else is a bundle this
    /// host cannot open, and saying so at the loader is the honest place.
    #[test]
    fn an_unnamed_architecture_falls_through_only_to_a_payload() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let bundle = root.path().join(format!("Odd{BUNDLE_SUFFIX}"));
        let odd = bundle.join(CONTENTS_DIR).join("sparc-solaris");
        std::fs::create_dir_all(&odd).expect("the odd directory");
        std::fs::write(odd.join("readme.txt"), b"not a payload").expect("the note");
        assert_eq!(payload(&bundle), None, "a text file is not a payload");

        let real = odd.join(format!("Odd.{PAYLOAD_EXTENSION}"));
        std::fs::write(&real, b"payload").expect("the payload");
        assert_eq!(payload(&bundle), Some(real));
    }
}
