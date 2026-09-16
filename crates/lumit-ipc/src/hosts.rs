//! The three strings each host owns, reserved against each other.
//!
//! # In plain terms
//!
//! A host's endpoint prefix, its broker executable's name and the environment
//! variable that overrides that executable are the host's **identity**, and
//! each host declares its own three as plain constants of its own - this crate
//! never hands them out. What it does hold is the reservation: the list of
//! which three belong to whom, so that "no two hosts share an endpoint prefix
//! or a broker-executable environment variable" is a thing a test can fail on
//! rather than a thing somebody notices.
//!
//! A collision would be quiet and nasty in both directions. One prefix for two
//! hosts puts both brokers' endpoints in one namespace, where two brokers with
//! the same identifier are the same socket. One environment variable for two
//! hosts lets a test that redirects the OFX broker redirect the LFX broker too,
//! which is a path to somebody else's code arriving from somewhere nobody
//! looked (docs/impl/lfx.md §3.1).
//!
//! Each host's own test asserts its constants are the ones reserved here, so a
//! rename in one place and not the other fails in that host's suite. The
//! reservation for LFX is here before the host is, which is the point: a name
//! reserved once the collision has shipped is a name reserved too late.
//!
//! ponytail: this is a reservation with a test behind it, not a mechanism.
//! [`crate::pipe_name`] and [`crate::broker_exe`] take the strings as free
//! arguments, so nothing here stops a fourth host passing `lumit-ofx` and
//! sharing the OFX namespace - what holds it up is that host's own
//! `this_hosts_three_strings_are_the_ones_reserved_for_it`, which its author
//! has to remember to write. Making the reservation the *type* -
//! `pipe_name(host: HostEndpoint, ..)`, each `ipc/identity.rs` holding the one
//! [`HostEndpoint`] it is - would make an unreserved prefix stop compiling
//! instead, and is a change to three hosts and three brokers rather than to
//! this file.

/// One host's three strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostEndpoint {
    /// The name this reservation goes by in a **test failure** - "OFX",
    /// "audio", "LFX". Nothing in a report line reads it; it is here so that a
    /// suite failing on a collision names the two hosts that collided rather
    /// than two prefixes the reader has to look up.
    pub host: &'static str,
    /// The endpoint prefix: a pipe is `{prefix}-{identifier}.pipe` on Windows
    /// and `{prefix}-{identifier}.sock` elsewhere (see [`crate::pipe_name`]).
    pub prefix: &'static str,
    /// The broker executable's name without a platform extension.
    pub broker_exe_stem: &'static str,
    /// The environment variable that overrides where that executable is.
    pub broker_exe_env: &'static str,
}

/// The OpenFX host (`lumit-ofx`).
pub const OFX: HostEndpoint = HostEndpoint {
    host: "OFX",
    prefix: "lumit-ofx",
    broker_exe_stem: "lumit-ofx-broker",
    broker_exe_env: "LUMIT_OFX_BROKER",
};

/// The audio-plugin host (`lumit-aplug`), CLAP and VST3.
pub const APLUG: HostEndpoint = HostEndpoint {
    host: "audio",
    prefix: "lumit-aplug",
    broker_exe_stem: "lumit-aplug-broker",
    broker_exe_env: "LUMIT_APLUG_BROKER",
};

/// The LFX host (`lumit-lfx`), reserved ahead of the crate that will claim it.
pub const LFX: HostEndpoint = HostEndpoint {
    host: "LFX",
    prefix: "lumit-lfx",
    broker_exe_stem: "lumit-lfx-broker",
    broker_exe_env: "LUMIT_LFX_BROKER",
};

/// Every reservation, in the order the hosts arrived.
pub const RESERVED: &[HostEndpoint] = &[OFX, APLUG, LFX];
