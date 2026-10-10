//! `lumit-relay [port] [kilobytes a second]`: be a relay for shared projects
//! on this machine, on the usual port unless another is given, until it is
//! stopped. The second number holds each host and guest to that speed once
//! a project's worth has passed, which lets edits through and makes footage
//! not worth sending this way.

use lumit_relay::{serve, Limits, DEFAULT_PORT};
use std::io::Write;
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

/// Say one line to whoever started this. A relay left running with nobody
/// listening to it says it to nobody, and carries on.
fn say(line: &str) {
    let _ = writeln!(std::io::stderr(), "lumit-relay: {line}");
}

fn main() -> ExitCode {
    let asked = std::env::args().nth(1);
    let port = match asked.as_deref().map(str::parse::<u16>) {
        None => DEFAULT_PORT,
        Some(Ok(port)) => port,
        Some(Err(_)) => {
            say("usage: lumit-relay [port] [kilobytes a second]");
            return ExitCode::FAILURE;
        }
    };
    let rate = match std::env::args().nth(2).map(|rate| rate.parse::<u64>()) {
        None => 0,
        Some(Ok(kilobytes)) => kilobytes.saturating_mul(1024),
        Some(Err(_)) => {
            say("usage: lumit-relay [port] [kilobytes a second]");
            return ExitCode::FAILURE;
        }
    };
    let limits = Limits {
        rate,
        ..Limits::default()
    };
    // Every address. Where one listener takes IPv4 and IPv6 both, the
    // second is refused and not needed.
    let listeners: Vec<TcpListener> = [
        TcpListener::bind((Ipv6Addr::UNSPECIFIED, port)),
        TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)),
    ]
    .into_iter()
    .filter_map(Result::ok)
    .collect();
    if listeners.is_empty() {
        say(&format!("port {port} could not be listened on"));
        return ExitCode::FAILURE;
    }
    say(&format!("listening on port {port}"));
    match serve(&listeners, limits, &AtomicBool::new(false)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            say(&why.to_string());
            ExitCode::FAILURE
        }
    }
}
