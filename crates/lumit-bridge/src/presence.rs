//! Lumit in the person's Discord status.
//!
//! Discord listens on a local pipe for programs that want to say what they are
//! doing. One thread here holds that pipe and tells it what the frontend last
//! asked for. Closing the pipe is what takes the status down, so quitting
//! Lumit needs no goodbye.

use std::io::{self, Read, Write};
use std::sync::{Mutex, Once};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;

/// Lumit's application id on Discord. Not a secret, every status carries it.
const CLIENT_ID: &str = "1558750922316976188";

/// How often the thread looks at what is wanted. Discord takes 5 updates in
/// 20 seconds, and this is also how soon a Discord started after Lumit is
/// found.
const PACE: Duration = Duration::from_secs(5);

/// Discord refuses a line shorter than 2 characters or longer than 128.
const LINE_MIN: usize = 2;
const LINE_MAX: usize = 128;

/// The two lines under Lumit's name. Both `None` says only that Lumit is open.
#[derive(Clone, PartialEq)]
pub(crate) struct Status {
    pub project: Option<String>,
    pub comp: Option<String>,
}

/// What the frontend last asked for. `None` is no status at all.
static WANTED: Mutex<Option<Status>> = Mutex::new(None);

static THREAD: Once = Once::new();

/// Show `status`, or take Lumit out of the status with `None`.
///
/// The thread starts the first time there is something to show, so with the
/// setting off Lumit never goes near Discord.
pub(crate) fn set(status: Option<Status>) {
    let show = status.is_some();
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = status;
    if show {
        THREAD.call_once(|| {
            // No thread means no status, which is not worth an error.
            let _ = std::thread::Builder::new()
                .name("discord-status".to_owned())
                .spawn(run);
        });
    }
}

fn run() {
    // When Lumit turned up in the status, for Discord's elapsed clock.
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let mut pipe: Option<Pipe> = None;
    let mut shown: Option<Status> = None;
    let mut sent = 0u64;
    loop {
        let wanted = WANTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match &wanted {
            None => pipe = None,
            Some(status) => {
                if pipe.is_none() {
                    pipe = connect();
                    shown = None;
                }
                if shown != wanted {
                    if let Some(open) = &mut pipe {
                        sent += 1;
                        if exchange(open, 1, &activity(status, since, sent)).is_err() {
                            pipe = None;
                        }
                    }
                }
            }
        }
        shown = if pipe.is_some() { wanted } else { None };
        std::thread::sleep(PACE);
    }
}

/// Discord's `SET_ACTIVITY` command for `status`. A line with nothing to say
/// is left out, as Discord refuses an empty one.
fn activity(status: &Status, since: u64, nonce: u64) -> serde_json::Value {
    let mut activity = json!({ "timestamps": { "start": since } });
    for (key, name) in [("details", &status.project), ("state", &status.comp)] {
        if let Some(name) = name.as_deref().filter(|name| !name.trim().is_empty()) {
            // Cut and padded to the lengths Discord takes.
            let cut: String = name.chars().take(LINE_MAX).collect();
            activity[key] = format!("{cut:<width$}", width = LINE_MIN).into();
        }
    }
    json!({
        "cmd": "SET_ACTIVITY",
        "nonce": nonce.to_string(),
        "args": { "pid": std::process::id(), "activity": activity },
    })
}

/// The first Discord that answers. A second Discord on the machine takes the
/// next number, up to 9.
fn connect() -> Option<Pipe> {
    (0..10).find_map(|n| {
        let mut pipe = open(n)?;
        let hello = json!({ "v": 1, "client_id": CLIENT_ID });
        exchange(&mut pipe, 0, &hello).ok().map(|()| pipe)
    })
}

/// Send one frame and read Discord's answer to it.
///
/// A frame is the opcode and the body's length, both little-endian `u32`s,
/// then the JSON. The answer is read and thrown away, since Discord's end of
/// the pipe would fill up if nobody did.
//
// ponytail: the read has no timeout, so a Discord that hangs parks this thread
// and the status stops following. A read timeout on the pipe if it ever shows.
fn exchange(pipe: &mut Pipe, opcode: u32, body: &serde_json::Value) -> io::Result<()> {
    let body = body.to_string();
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body.as_bytes());
    pipe.write_all(&frame)?;

    let mut head = [0u8; 8];
    pipe.read_exact(&mut head)?;
    let [a, b, c, d, e, f, g, h] = head;
    let length = u64::from(u32::from_le_bytes([e, f, g, h]));
    io::copy(&mut Read::by_ref(pipe).take(length), &mut io::sink())?;
    // Opcode 2 is Discord closing on us, which is its answer to an id it does
    // not know.
    if u32::from_le_bytes([a, b, c, d]) == 2 {
        return Err(io::ErrorKind::ConnectionRefused.into());
    }
    Ok(())
}

#[cfg(windows)]
type Pipe = std::fs::File;

#[cfg(windows)]
fn open(n: u32) -> Option<Pipe> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!(r"\\.\pipe\discord-ipc-{n}"))
        .ok()
}

#[cfg(unix)]
type Pipe = std::os::unix::net::UnixStream;

/// Discord's socket is in the runtime folder or the temp one. The Flatpak and
/// the Snap each keep theirs a folder down.
#[cfg(unix)]
fn open(n: u32) -> Option<Pipe> {
    use std::path::PathBuf;
    ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .chain([PathBuf::from("/tmp")])
        .flat_map(|dir| ["", "app/com.discordapp.Discord", "snap.discord"].map(|sub| dir.join(sub)))
        .find_map(|dir| Pipe::connect(dir.join(format!("discord-ipc-{n}"))).ok())
}
