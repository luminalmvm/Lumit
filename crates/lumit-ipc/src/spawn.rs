//! Finding the broker executable, and starting it quietly.
//!
//! # In plain terms
//!
//! Two small things every host does identically before it has a broker to talk
//! to: work out where the second program is, and ask the operating system not
//! to put a console window in front of the editor when it starts.
//!
//! Neither is parameterised by anything but the host's own two strings - the
//! executable's file name and the environment variable that overrides it - and
//! those are arguments rather than constants for the reason [`crate::pipe`]
//! gives about the endpoint prefix: baked in, `LUMIT_OFX_BROKER` would redirect
//! every host's child, including the ones running code Lumit has never seen
//! before (docs/impl/lfx.md §3.1).

use std::path::PathBuf;
use std::process::Command;

/// Where the broker executable is: beside Lumit's own, which is where every
/// packaging step puts it.
///
/// `exe_name` is the file name the calling host's packaging writes -
/// `lumit-ofx-broker`, or the same with `.exe` on Windows - and `env_var` is
/// that host's own override, for a test or for a developer running from a build
/// tree. An override is taken as given and is not checked here: it is a
/// developer's own machine speaking, and a path that is not there fails at the
/// spawn with the operating system's own words.
#[must_use]
pub fn broker_exe(exe_name: &str, env_var: &str) -> PathBuf {
    if let Some(override_path) = std::env::var_os(env_var) {
        return PathBuf::from(override_path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(exe_name)))
        .unwrap_or_else(|| PathBuf::from(exe_name))
}

/// Start the broker with no console window of its own.
///
/// A broker is a console program and Lumit is a windowed one, so on Windows
/// every spawn opens a console window in front of the editor - one per plugin
/// file, all at once, during the start-up scan. `CREATE_NO_WINDOW` gives the
/// child no console at all instead. Nothing is lost by it: the protocol was
/// never on the child's standard streams (see [`crate::pipe`]), and its output
/// is already sent to nowhere.
pub fn no_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`, from winbase.h.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
