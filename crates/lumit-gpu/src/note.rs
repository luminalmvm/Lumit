//! Console diagnostics that cannot take the application down.
//!
//! # In plain terms
//!
//! `eprintln!` looks harmless, and is not. It writes to the console the process
//! was started with, and when nothing is listening any more — a Windows GUI
//! build whose console has gone, a `flutter run` that has been closed — the
//! write fails, and the standard macros answer a failed write by panicking.
//! This crate reports a lost device and an uncaptured wgpu error from inside
//! callbacks the driver invokes, where a panic is the end of the session.
//!
//! A note nobody can hear is not worth a crash. [`note!`] writes the same line
//! and drops the error. `lumit-bridge` carries the same macro for the same
//! reason; the two crates share no dependency, and eight lines twice is a
//! cheaper answer than an edge from this crate to another.

use std::fmt::Arguments;
use std::io::Write;

/// Write one diagnostic line to the error stream, ignoring a failed write.
///
/// Takes the same arguments as `eprintln!`.
macro_rules! note {
    ($($arg:tt)*) => {
        $crate::note::write_line(&mut std::io::stderr(), format_args!($($arg)*))
    };
}

/// The write itself, with the sink passed in so the "a closed pipe does not
/// panic" rule can be tested without closing the process's own console.
pub(crate) fn write_line(out: &mut impl Write, args: Arguments<'_>) {
    let _ = writeln!(out, "{args}");
}

#[cfg(test)]
mod tests {
    use super::write_line;
    use std::io::{Error, ErrorKind, Write};

    /// A console that has gone away: every write fails the way a pipe with no
    /// reader fails on Windows.
    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(Error::new(
                ErrorKind::BrokenPipe,
                "The pipe is being closed",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(Error::new(
                ErrorKind::BrokenPipe,
                "The pipe is being closed",
            ))
        }
    }

    #[test]
    fn a_closed_console_does_not_panic() {
        write_line(&mut ClosedPipe, format_args!("device lost"));
    }
}
