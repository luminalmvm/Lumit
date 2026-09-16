//! The three strings that are this host's own.
//!
//! # In plain terms
//!
//! The transport lives in `lumit-ipc` and is shared with the OFX host and with
//! LFX. These three are what it is *not* shared on: the prefix every audio
//! broker's endpoint is named after, the file name of the second program, and
//! the environment variable that says where that program is when a developer is
//! running from a build tree.
//!
//! They are constants here rather than arguments there because they are this
//! host's identity. One prefix shared between two hosts puts both brokers'
//! endpoints in one namespace; one environment variable shared between two lets
//! a redirect meant for this host's broker start somebody else's code in the
//! other's (docs/impl/lfx.md §3.1). `lumit_ipc::hosts` holds the reservation
//! that keeps the three apart, and
//! `this_hosts_three_strings_are_the_ones_reserved_for_it` in `src/tests.rs` is
//! this host's half of it.

/// The endpoint prefix: `lumit-aplug-{identifier}.pipe` on Windows, and the
/// same with `.sock` in the temporary directory elsewhere.
pub const HOST_PREFIX: &str = "lumit-aplug";

/// The environment variable that overrides where the broker executable is,
/// for a test or for a developer running from a build tree.
pub const BROKER_EXE_ENV: &str = "LUMIT_APLUG_BROKER";

/// The broker executable's file name.
#[must_use]
pub fn broker_exe_name() -> &'static str {
    if cfg!(windows) {
        "lumit-aplug-broker.exe"
    } else {
        "lumit-aplug-broker"
    }
}
