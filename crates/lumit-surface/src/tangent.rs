//! The Tangent thread: one TCP connection to the Hub on this machine, kept
//! while panels are on.
//!
//! Runs on a thread of its own, named `tangent`. It writes our control and
//! panel map files, then connects, and connects again with a growing wait
//! whenever the Hub is not there or goes away. The socket is read with a
//! short timeout, so between reads it can see that panels were turned off or
//! that the interface has new values for the displays. Those arrive through
//! a latest-wins mailbox, read under its lock and then let go before any
//! write.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::exchange::Exchange;
use crate::tipc::{self, Framer};
use crate::{Shared, TangentStatus};

/// Where the Hub listens. Only ever this machine.
pub(crate) const HUB: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 64246);

/// How long a read waits before the thread looks round.
const TICK: Duration = Duration::from_millis(40);
/// The first wait after the Hub was not there, doubled each time up to
/// [`LONGEST_WAIT`].
const FIRST_WAIT: Duration = Duration::from_millis(500);
const LONGEST_WAIT: Duration = Duration::from_secs(5);
const CONNECT_WAIT: Duration = Duration::from_millis(500);
const WRITE_WAIT: Duration = Duration::from_secs(1);

/// The thread's body. Returns once `stop` is set.
pub(crate) fn run(shared: &Shared, stop: &AtomicBool, folder: &Path) {
    // A folder the Hub cannot read leaves the panels blank but still
    // connected, which says more than refusing to start.
    let _ = crate::xml::write_all(folder);
    let sys_dir = folder.to_string_lossy().into_owned();
    let mut wait = FIRST_WAIT;
    while !stop.load(Ordering::Relaxed) {
        shared.set_status(TangentStatus::Waiting, stop);
        if let Ok(stream) = TcpStream::connect_timeout(&HUB, CONNECT_WAIT) {
            wait = FIRST_WAIT;
            talk(shared, stop, stream, &sys_dir);
            continue;
        }
        pause(stop, wait);
        wait = (wait * 2).min(LONGEST_WAIT);
    }
}

/// Wait `long`, in short steps so a stop is seen.
fn pause(stop: &AtomicBool, long: Duration) {
    let mut left = long;
    while !left.is_zero() && !stop.load(Ordering::Relaxed) {
        let step = left.min(TICK);
        std::thread::sleep(step);
        left -= step;
    }
}

/// One connection, until it drops or panels are turned off.
fn talk(shared: &Shared, stop: &AtomicBool, mut stream: TcpStream, sys_dir: &str) {
    if stream.set_read_timeout(Some(TICK)).is_err()
        || stream.set_write_timeout(Some(WRITE_WAIT)).is_err()
    {
        return;
    }
    let _ = stream.set_nodelay(true);
    let mut exchange = Exchange::default();
    let mut framer = Framer::default();
    let mut buf = [0u8; 4096];
    // Nothing is shown until the Hub has said hello.
    let mut greeted = false;
    let mut seen = u64::MAX;
    while !stop.load(Ordering::Relaxed) {
        let (generation, shown) = shared.shown();
        let mut out = Vec::new();
        let mut events = Vec::new();
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let mut commands = Vec::new();
                let read = framer.push(buf.get(..n).unwrap_or_default(), |body| {
                    commands.extend(tipc::decode(body));
                });
                if read.is_err() {
                    return;
                }
                for command in commands {
                    if let Some(panels) =
                        exchange.on(command, &shown, sys_dir, &mut out, &mut events)
                    {
                        greeted = true;
                        seen = u64::MAX;
                        shared.set_status(TangentStatus::Connected { panels }, stop);
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => return,
        }
        if greeted && generation != seen {
            seen = generation;
            exchange.refresh(&shown, &mut out);
        }
        if !out.is_empty() && stream.write_all(&tipc::encode(&out)).is_err() {
            return;
        }
        for event in events {
            (shared.tell)(event);
        }
    }
}
