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
pub(crate) fn watch() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{path, record, record_to, watch, CAP};

    fn contents() -> String {
        std::fs::read_to_string(path()).unwrap_or_default()
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
