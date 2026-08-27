//! Console diagnostics that cannot take the application down.
//!
//! # In plain terms
//!
//! `println!` looks harmless, and is not. It writes to the console the process
//! was started with, and when nothing is listening any more — a Windows GUI
//! build whose console has gone, a `flutter run` that has been closed — the
//! write fails; the standard macros answer a failed write by panicking. Inside
//! a `#[frb(sync)]` call that panic crosses into Dart as a `PanicException`, so
//! switching the render-time measurements *off* reported a crash instead of
//! switching anything off.
//!
//! A note nobody can hear is not worth a crash. [`note!`] writes the same line
//! and drops the error: the diagnostic is a courtesy, the edit session is not.
//! Every diagnostic line in this crate goes through it, and
//! `tests/no_panicking_prints.rs` fails the build if a `println!` creeps back.

use std::fmt::Arguments;
use std::io::Write;

/// Write one diagnostic line to the error stream, ignoring a failed write.
///
/// Takes the same arguments as `println!`.
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
        write_line(&mut ClosedPipe, format_args!("render profiling off"));
    }

    #[test]
    fn the_line_reaches_a_console_that_is_listening() {
        let mut out = Vec::new();
        write_line(&mut out, format_args!("measured frame {}", 12));
        assert_eq!(out, b"measured frame 12\n");
    }
}
