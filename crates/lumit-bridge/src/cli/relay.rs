//! `lumit-cli mcp` with no project: the program on standard input and output
//! talks to the Lumit that is open, through its live endpoint.
//!
//! Lines go across as they are, both ways. The endpoint's file says where
//! Lumit is and holds the secret both ends prove they have
//! (`lumit_script::live`), and there is a file only while the person lets
//! other programs drive Lumit.

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use lumit_script::live::{self, Unreached};
use serde_json::Value;

/// Relay standard input to Lumit and Lumit's answers to standard output,
/// until the program closes its end and has its last answers, or Lumit lets
/// go.
pub fn relay() -> Result<(), String> {
    let file = lumit_project::live_endpoint_file().ok_or_else(off)?;
    let link = live::connect(&file).map_err(|why| match why {
        Unreached::Off => off(),
        Unreached::Busy => "Another program is driving Lumit.".to_owned(),
        Unreached::Denied | Unreached::NotLumit => {
            "Lumit could not be reached safely. Turn Let other programs drive Lumit off and on \
             again in Settings."
                .to_owned()
        }
    })?;
    note!("Connected to Lumit");
    let live::Link { mut from, mut to } = link;
    // Requests still to be answered, so a program that closes its end
    // straight after asking still hears back.
    let waiting = Arc::new(AtomicUsize::new(0));
    let answered = waiting.clone();
    std::thread::spawn(move || {
        let mut output = std::io::stdout().lock();
        let mut line = String::new();
        loop {
            line.clear();
            match from.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if output
                .write_all(line.as_bytes())
                .and_then(|()| output.flush())
                .is_err()
            {
                break;
            }
            let _ = answered.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
        }
        note!("Lumit let go of the connection.");
        std::process::exit(1);
    });
    let mut input = std::io::stdin().lock();
    let mut line = String::new();
    loop {
        line.clear();
        let read = input
            .read_line(&mut line)
            .map_err(|e| format!("Standard input could not be read: {e}."))?;
        if read == 0 {
            break;
        }
        // Without the byte order mark some shells put before the first line.
        let text = line.trim().trim_start_matches('\u{feff}');
        if text.is_empty() {
            continue;
        }
        if is_request(text) {
            waiting.fetch_add(1, Ordering::Relaxed);
        }
        writeln!(to, "{text}").map_err(|_| "Lumit let go of the connection.".to_owned())?;
    }
    while waiting.load(Ordering::Relaxed) > 0 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}

fn off() -> String {
    "Lumit isn't open to other programs. Turn on Let other programs drive Lumit in Settings."
        .to_owned()
}

/// Whether `line` asks something that will be answered: it has an id and a
/// method, or is not JSON, which is answered with an error.
fn is_request(line: &str) -> bool {
    serde_json::from_str::<Value>(line).map_or(true, |message| {
        message.get("id").is_some() && message.get("method").is_some()
    })
}
