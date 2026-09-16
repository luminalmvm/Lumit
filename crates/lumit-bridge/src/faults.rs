//! The file a crash leaves behind.
//!
//! # In plain terms
//!
//! When something goes badly wrong in the engine, the code says so with
//! [`note!`](crate::note) — which writes to *standard error*. That is fine when
//! Lumit was started from a terminal, and it is nothing at all when it was
//! started the way people actually start it: a double-click on a windowed
//! Windows build has no console attached, so every one of those lines is
//! written to a handle that goes nowhere.
//!
//! That is why the render worker's crash net could catch a fault, name it, and
//! still leave a bug report reading only "it froze and closed". The line was
//! printed. Nobody could ever have seen it.
//!
//! So the worst lines are also appended to a file: a fault the crash net
//! caught, and — through a process-wide panic hook — any panic anywhere,
//! including the ones that are *not* caught and take the process with them.
//! Whatever ends the session is named on disk before it ends, and the next
//! occurrence can be read rather than guessed at.
//!
//! It is deliberately not a logging framework. One file, appended to, capped so
//! it cannot fill a disk, and never a reason to fail: every write here is
//! allowed to go wrong quietly, because a diagnostic that can break the thing it
//! is diagnosing is worse than none.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;

/// The file's name inside Lumit's cache directory (`lumit_project::cache_dir`),
/// or inside the system's temporary directory on a machine with no home.
const FILE: &str = "lumit-diagnostics.log";

/// Past this many bytes the file starts again. A crash report wants the *last*
/// faults, and a session that faults in a loop would otherwise write until the
/// disk was full.
const CAP: u64 = 256 * 1024;

/// Where the diagnostics go. Printed once at startup so a bug report can say
/// where to look.
pub(crate) fn path() -> PathBuf {
    lumit_project::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(FILE)
}

/// Append one line, with the seconds since the epoch in front of it so two
/// faults can be told apart and matched against when the editor was used.
///
/// Never fails, never panics, never blocks anything waiting on it.
pub(crate) fn record(line: &str) {
    record_to(&path(), line);
}

/// The write itself, with the file passed in so the size cap can be tested
/// without filling the one a real session is writing to.
fn record_to(file: &std::path::Path, line: &str) {
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Started afresh once it is too big. Checked before the open rather than
    // after the write, so the file the line lands in is the one that keeps it.
    let over = std::fs::metadata(file)
        .map(|m| m.len() > CAP)
        .unwrap_or(false);
    // The cache directory may not exist yet on a first run.
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .append(!over)
        .write(true)
        .truncate(over)
        .open(file);
    // Formatted first and written in **one** call. `writeln!` issues a write per
    // piece of the format string, and two threads faulting at once then hand the
    // file each other's halves — which is precisely the moment the file has to
    // be readable, since a fault on one thread is often what pushed the other
    // over. An append-mode write of a whole line is what the OS keeps together.
    // One record is one line, whatever it says: a panic's own text runs to two
    // lines ("panicked at …:" and then the message), and a file where one entry
    // is sometimes one line and sometimes two cannot be read with `findstr`.
    let flat = line.replace(['\r', '\n'], " | ");
    if let Ok(mut out) = opened {
        let _ = out.write_all(format!("[{when}] {flat}\n").as_bytes());
    }
}

/// Install the process-wide panic hook, once for the life of the process.
///
/// **The case this exists for.** The render worker's crash net catches the
/// panics that happen inside a turn, and those are the ones we know how to
/// recover from. A panic anywhere else — a background thread, a callback the
/// platform drives, an unwind that reaches an FFI boundary and aborts — ends the
/// process, and Dart reports it as nothing more informative than the device
/// being lost. The hook runs *before* the unwind, so the message and the line it
/// came from are on disk even when nothing survives to write them afterwards.
///
/// The hook that was already installed is called after this one, so nothing that
/// depended on the default output loses it.
///
/// [`arm_on_load`] has already called this as the library loaded, so the render
/// worker's own call arms nothing and costs nothing.
pub(crate) fn watch() {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>").to_string();
            let place = match info.location() {
                Some(at) => format!("{}:{}:{}", at.file(), at.line(), at.column()),
                None => "an unknown place".to_string(),
            };
            record(&format!("panic on thread {name} at {place}: {info}"));
            previous(info);
        }));
    });
}

/// Installed once, whoever asks and however often.
static HOOK: Once = Once::new();

/// Arm the hook as the library loads, before any of Lumit's own code runs.
///
/// The render worker used to be the only one who armed it, and a worker only
/// exists once a project is open, so a panic on the way up took the process
/// with it and left the file empty. The platform walks this list of
/// initialisers when it loads the library, the same way a C++ global is built,
/// which is the earliest moment there is.
#[used]
#[cfg_attr(target_os = "windows", link_section = ".CRT$XCU")]
#[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
#[cfg_attr(
    any(target_os = "linux", target_os = "android", target_os = "freebsd"),
    link_section = ".init_array"
)]
static ARM_ON_LOAD: extern "C" fn() = arm_on_load;

extern "C" fn arm_on_load() {
    watch();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{path, record, record_to, watch, CAP, HOOK};

    fn contents() -> String {
        std::fs::read_to_string(path()).unwrap_or_default()
    }

    /// The name of the test below, as the harness filters on it.
    const NO_WORKER_TEST: &str = "faults::tests::the_hook_is_armed_before_any_worker_exists";

    /// Set on the child run, and holding the marker the child panics with.
    const CHILD: &str = "LUMIT_FAULTS_NO_WORKER_CHILD";

    /// **The regression a start-up crash needs.** The hook must be armed before
    /// anything asks for it: the worker arms it too, and a worker only exists
    /// once a project is open, so a panic on the way up wrote nothing at all.
    ///
    /// Proved in a child process running this one test alone. Every other test
    /// in this binary shares the process and one of them calls `watch`, so
    /// "nobody has armed it yet" is only true on a run of its own.
    #[test]
    fn the_hook_is_armed_before_any_worker_exists() {
        if let Ok(marker) = std::env::var(CHILD) {
            assert!(
                HOOK.is_completed(),
                "nothing has called watch in this process, so the hook can only \
                 have been armed as the library loaded"
            );
            let handle = std::thread::Builder::new()
                .name("lumit-faults-no-worker".into())
                .spawn(move || panic!("{marker}"))
                .expect("a thread");
            assert!(handle.join().is_err(), "the thread was meant to panic");
            return;
        }
        let marker = format!("no-worker-{:?}", std::time::Instant::now());
        let child = std::process::Command::new(std::env::current_exe().expect("this test binary"))
            .args(["--exact", NO_WORKER_TEST, "--test-threads=1"])
            .env(CHILD, &marker)
            .output()
            .expect("the test binary runs again");
        let said = String::from_utf8_lossy(&child.stdout).to_string();
        assert!(
            said.contains("1 passed"),
            "the child run did not pass {NO_WORKER_TEST}: {said}"
        );
        assert!(
            contents().contains(&marker),
            "a panic before any worker existed did not reach {:?}",
            path()
        );
    }

    /// A recorded line is on disk, with a time in front of it.
    #[test]
    fn a_recorded_line_can_be_read_back() {
        let marker = format!("marker-{:?}", std::time::Instant::now());
        record(&marker);
        let text = contents();
        assert!(
            text.contains(&marker),
            "the line never reached {:?}",
            path()
        );
        assert!(
            text.lines()
                .any(|l| l.starts_with('[') && l.contains(&marker)),
            "a line with no time on it cannot be placed in a session"
        );
    }

    /// **The regression the open crash report needs.** A panic on a thread
    /// nobody is catching for must name itself on disk. Before the hook, a
    /// process that ended this way left the file untouched and the report read
    /// "lost connection to device" and nothing else.
    #[test]
    fn a_panic_names_itself_even_where_nothing_catches_it() {
        watch();
        // Twice, because the hook must be installed once however many times it
        // is asked for — the worker calls it on every start.
        watch();
        let marker = format!("fault-{:?}", std::time::Instant::now());
        let panicking = marker.clone();
        let handle = std::thread::Builder::new()
            .name("lumit-faults-test".into())
            .spawn(move || panic!("{panicking}"))
            .expect("a thread");
        assert!(handle.join().is_err(), "the thread was meant to panic");
        let text = contents();
        assert!(
            text.contains(&marker),
            "the panic did not reach {:?}",
            path()
        );
        assert!(
            text.contains("lumit-faults-test"),
            "the fault must name the thread it happened on"
        );
        assert!(
            text.contains("faults.rs:"),
            "the fault must name the line it came from"
        );
    }

    /// The file cannot grow without bound: a session faulting in a loop starts
    /// the file again rather than filling the disk.
    #[test]
    fn the_file_starts_again_once_it_is_too_big() {
        // Its own file: filling the real one would wipe the markers the other
        // tests in here are reading back at the same moment.
        let file = std::env::temp_dir().join("lumit-diagnostics-cap-test.log");
        let _ = std::fs::remove_file(&file);
        let filler = "x".repeat(4096);
        for _ in 0..(CAP / 4096 + 4) {
            record_to(&file, &filler);
        }
        let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        assert!(len <= CAP + 8192, "the file grew to {len} bytes");
        let _ = std::fs::remove_file(&file);
    }
}
