//! The override [`lumit_ipc::broker_exe`] reads, in a process of its own.
//!
//! # In plain terms
//!
//! Pinning "an override wins, and without one the broker is beside the host"
//! means setting an environment variable, and setting one is the only thing in
//! this crate that *writes* the process environment. Writing it is not a local
//! act: `setenv` may move the block `getenv` is reading, so a test that writes
//! races every test in the same binary that reads - and the unit suite is full
//! of readers, since `pipe_name` asks for the temporary directory on every call
//! and `broker_exe` asks for its own variable. The failure that buys is a rare
//! abort with no test named against it, which is the worst kind to be handed on
//! somebody else's machine.
//!
//! Hence a second binary with nothing else in it. The two tests that are in it
//! still run on separate threads, and one of them writes, so they take a lock
//! between them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, PoisonError};

use lumit_ipc::broker_exe;

/// Held across every read of and every write to the environment here.
///
/// The poison is deliberately taken as read: a test that failed while holding
/// this would otherwise fail the other one too, for a reason that is not its
/// own. Nothing is guarded but the environment, and a panic leaves that no less
/// consistent than it was.
static ENV: Mutex<()> = Mutex::new(());

/// A developer running from a build tree, or a test, points the host at an
/// executable somewhere else; what is there is not checked, because it is a
/// developer's own machine speaking.
#[test]
fn the_broker_executable_is_the_override_when_one_is_set() {
    const VAR: &str = "LUMIT_IPC_TEST_BROKER_OVERRIDE";
    let found = {
        let _guard = ENV.lock().unwrap_or_else(PoisonError::into_inner);
        std::env::set_var(VAR, "/somewhere/else/a-broker");
        let found = broker_exe("lumit-ofx-broker", VAR);
        std::env::remove_var(VAR);
        found
    };
    assert_eq!(found, std::path::PathBuf::from("/somewhere/else/a-broker"));
}

/// With nothing set, the answer is the file beside the running program, which
/// is where every packaging step puts it.
#[test]
fn the_broker_executable_is_beside_the_host_when_nothing_overrides_it() {
    const VAR: &str = "LUMIT_IPC_TEST_BROKER_UNSET";
    let (found, here) = {
        let _guard = ENV.lock().unwrap_or_else(PoisonError::into_inner);
        std::env::remove_var(VAR);
        let found = broker_exe("lumit-ofx-broker", VAR);
        (found, std::env::current_exe().expect("this test binary"))
    };
    assert_eq!(
        found.file_name().and_then(std::ffi::OsStr::to_str),
        Some("lumit-ofx-broker")
    );
    assert_eq!(found.parent(), here.parent(), "beside the running program");
}
