//! `lumit-relay [port]`: be a relay for shared projects on this machine, on
//! the usual port unless another is given, until it is stopped.

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
            say("usage: lumit-relay [port]");
            return ExitCode::FAILURE;
        }
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
    match serve(&listeners, Limits::default(), &AtomicBool::new(false)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            say(&why.to_string());
            ExitCode::FAILURE
        }
    }
}
