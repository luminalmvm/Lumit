//! The live endpoint: how a program on this computer reaches the Lumit that
//! is open, and the file that says where it is.
//!
//! # In plain terms
//!
//! It is off until the person turns on "Let other programs drive Lumit". Then
//! Lumit listens on a local socket, a named pipe on Windows and a Unix socket
//! in a folder only this user can open elsewhere, and never on a network
//! port. The name is random, and the name and a secret go in a file only this
//! user can read. `lumit-cli mcp` reads the file, connects, and both ends
//! prove they hold the secret (`lumit_peer`) before a line of MCP is read.
//! One program at a time: a second is told so and let go.
//!
//! Every message is one line of JSON.
//!
//! ```text
//!   Lumit   ──► { "nonce": N_l }
//!   program ──► { "nonce": N_p, "proof": Proof::broker(secret, N_l) }
//!                 └─ Lumit checks this, and lets go of the program when it is wrong
//!   Lumit   ──► { "proof": Proof::host(secret, N_p) }
//!                 └─ the program checks this, and only then sends MCP
//! ```
//!
//! Lumit's side never waits on the other end for long: the socket does not
//! block, so turning the setting off or pressing Disconnect is heard within
//! a few milliseconds, whatever the program is doing.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::{Listener as _, Stream as _};
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerNonblockingMode, ListenerOptions, RecvHalf,
    SendHalf, Stream, ToFsName, ToNsName,
};
use lumit_peer::{Nonce, Proof, Token};
use serde::{Deserialize, Serialize};

pub use interprocess::local_socket::Listener;
pub use lumit_peer::Secret;

/// How long a program has to prove itself once it has connected.
const HANDSHAKE: Duration = Duration::from_secs(5);
/// How often Lumit's side looks again when nothing has arrived.
const TICK: Duration = Duration::from_millis(10);
/// The longest line taken, room for a long script. Anything longer ends the
/// connection, so a program cannot make Lumit hold more than this.
const MAX_LINE: usize = 32 << 20;

#[derive(Serialize, Deserialize)]
struct Opening {
    nonce: Nonce,
}

#[derive(Serialize, Deserialize)]
struct Hello {
    nonce: Nonce,
    proof: Proof,
}

#[derive(Serialize, Deserialize)]
struct Answer {
    proof: Proof,
}

/// What a program is told when another is connected already.
#[derive(Serialize, Deserialize)]
struct Refused {
    refused: String,
}

/// Start listening, and write `file` saying where and with what secret.
/// The listener is made first, so the file never names a socket that is not
/// there yet.
///
/// # Errors
///
/// No randomness, a folder or file that cannot be made, or a name that is
/// taken.
pub fn open(file: &Path) -> io::Result<(Listener, Secret)> {
    let dir = file
        .parent()
        .ok_or_else(|| io::Error::other("the endpoint file has no folder"))?;
    private_folder(dir)?;
    let token = Token::generate().map_err(io::Error::other)?.as_name();
    let secret = Secret::generate().map_err(io::Error::other)?;
    #[cfg(not(unix))]
    let name = format!("lumit-live-{token}");
    #[cfg(unix)]
    let name = socket_path(dir, &token)?;
    // Named pipes turn away other computers unless asked not to, and this
    // never asks.
    let options = if cfg!(windows) {
        ListenerOptions::new().name(name.as_str().to_ns_name::<GenericNamespaced>()?)
    } else {
        ListenerOptions::new().name(name.as_str().to_fs_name::<GenericFilePath>()?)
    };
    // Only the user who started Lumit may open the pipe. By default anyone
    // signed in to this computer could, and hold it so nobody else can.
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::local_socket::ListenerOptionsExt;
        use interprocess::os::windows::security_descriptor::SecurityDescriptor;
        let owner_only =
            widestring::U16CString::from_str("D:P(A;;GA;;;OW)").map_err(io::Error::other)?;
        options.security_descriptor(SecurityDescriptor::deserialize(&owner_only)?)
    };
    let listener = options
        .nonblocking(ListenerNonblockingMode::Both)
        .create_sync()?;
    write_private(file, &format!("{name}\n{}", secret.to_handover()))?;
    Ok((listener, secret))
}

/// Take `file` away if it still holds `secret`. Another Lumit that turned
/// the setting on since has written its own, and keeps it.
pub fn remove(file: &Path, secret: &Secret) {
    let held = std::fs::read_to_string(file).ok().and_then(|text| {
        let (_, held) = text.split_once('\n')?;
        Secret::read_from(held.as_bytes()).ok()
    });
    if held.as_ref() == Some(secret) {
        let _ = std::fs::remove_file(file);
    }
}

/// Make `dir`, readable by this user only.
fn private_folder(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        // A folder that was there already keeps its own mode otherwise.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    // The user's own local app data is private on Windows: the user,
    // SYSTEM and Administrators, inherited by what is made inside.
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// Where the socket goes on macOS and Linux. Its whole path has to fit in
/// 104 bytes on macOS and 108 on Linux, and the data folder alone can be
/// nearly that, so it goes in a short folder only this user can open: the
/// runtime folder on Linux, the temp folder macOS gives each user, or one of
/// our own in /tmp. Half the token is plenty, the secret is what keeps
/// others out.
#[cfg(unix)]
fn socket_path(data: &Path, token: &str) -> io::Result<String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::path::PathBuf;
    // The data folder was just made ours, so its owner is who we are.
    let uid = std::fs::metadata(data)?.uid();
    let private = |dir: &Path| {
        std::fs::symlink_metadata(dir)
            .is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o077 == 0)
    };
    let given = ["XDG_RUNTIME_DIR", "TMPDIR"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|dir| dir.is_absolute() && private(dir));
    let dir = match given {
        Some(dir) => dir,
        None => {
            let dir = PathBuf::from(format!("/tmp/lumit-{uid}"));
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
                _ => {}
            }
            // Anyone can make a folder in /tmp, so one made first by someone
            // else, or opened up to them, is not used.
            if !private(&dir) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is not private to this user", dir.display()),
                ));
            }
            dir
        }
    };
    let short = token.get(..16).unwrap_or(token);
    let path = dir.join(format!("lumit-live-{short}"));
    let path = path.to_string_lossy().into_owned();
    // Less one byte for the end of the string.
    let room = if cfg!(target_os = "linux") { 107 } else { 103 };
    if path.len() > room {
        return Err(io::Error::other(format!(
            "the socket path is {} bytes, more than the {room} a socket takes: {path}",
            path.len()
        )));
    }
    Ok(path)
}

/// Write `text` to `file`, readable by this user only.
fn write_private(file: &Path, text: &str) -> io::Result<()> {
    let _ = std::fs::remove_file(file);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(file)?.write_all(text.as_bytes())
}

/// Answer programs on `listener` until `stop` is set: one at a time, each
/// proving it holds `secret` first. `leave` set lets go of the program that
/// is connected, and is cleared when the next one connects. `attached` hears
/// true when a program has proved itself and false when it has gone, and
/// `answer` turns each of its lines into the line to send back.
pub fn serve(
    listener: &Listener,
    secret: &Secret,
    stop: &AtomicBool,
    leave: &AtomicBool,
    attached: &mut dyn FnMut(bool),
    answer: &mut dyn FnMut(&str) -> Option<String>,
) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok(stream) => {
                leave.store(false, Ordering::Relaxed);
                let gone = || stop.load(Ordering::Relaxed) || leave.load(Ordering::Relaxed);
                let mut wire = Wire {
                    stream,
                    got: Vec::new(),
                };
                if handshake(listener, &mut wire, secret, &gone).is_ok() {
                    attached(true);
                    let _ = attend(listener, &mut wire, &gone, answer);
                    attached(false);
                }
            }
            Err(_) => std::thread::sleep(TICK),
        }
    }
}

/// Lumit's half of the challenge. An error means the program is let go.
fn handshake(
    listener: &Listener,
    wire: &mut Wire,
    secret: &Secret,
    gone: &dyn Fn() -> bool,
) -> io::Result<()> {
    let ours = Nonce::generate().map_err(io::Error::other)?;
    wire.send(&json_line(&Opening { nonce: ours })?, gone)?;
    let deadline = Instant::now() + HANDSHAKE;
    let line = loop {
        if gone() || Instant::now() > deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        refuse_others(listener);
        match wire.line()? {
            Some(line) => break line,
            None => std::thread::sleep(TICK),
        }
    };
    let hello: Hello = serde_json::from_str(&line).map_err(io::Error::other)?;
    if !hello.proof.matches(&Proof::broker(secret, ours)) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let proof = Proof::host(secret, hello.nonce);
    wire.send(&json_line(&Answer { proof })?, gone)
}

/// The connected program's lines, each answered in turn, until it goes or is
/// let go.
fn attend(
    listener: &Listener,
    wire: &mut Wire,
    gone: &dyn Fn() -> bool,
    answer: &mut dyn FnMut(&str) -> Option<String>,
) -> io::Result<()> {
    while !gone() {
        refuse_others(listener);
        match wire.line()? {
            Some(line) if line.is_empty() => {}
            Some(line) => {
                if let Some(reply) = answer(&line) {
                    wire.send(&reply, gone)?;
                }
            }
            None => std::thread::sleep(TICK),
        }
    }
    Ok(())
}

/// Tell any other program that has connected that one is here already, and
/// let it go.
fn refuse_others(listener: &Listener) {
    while let Ok(stream) = listener.accept() {
        let mut wire = Wire {
            stream,
            got: Vec::new(),
        };
        if let Ok(line) = json_line(&Refused {
            refused: "busy".into(),
        }) {
            let _ = wire.send(&line, &|| false);
        }
    }
}

fn json_line(message: &impl Serialize) -> io::Result<String> {
    serde_json::to_string(message).map_err(io::Error::other)
}

/// One connection on Lumit's side, read a line at a time without waiting.
struct Wire {
    stream: Stream,
    /// What has come in past the last whole line.
    got: Vec<u8>,
}

impl Wire {
    /// The next whole line, or `None` when it has not all come yet. An error
    /// when the program has gone or sent a line past [`MAX_LINE`].
    fn line(&mut self) -> io::Result<Option<String>> {
        let mut chunk = [0_u8; 16 << 10];
        loop {
            if let Some(end) = self.got.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.got.drain(..=end).collect();
                return Ok(Some(String::from_utf8_lossy(&line).trim().to_owned()));
            }
            if self.got.len() > MAX_LINE {
                return Err(io::ErrorKind::InvalidData.into());
            }
            match self.stream.read(&mut chunk) {
                // A pipe on Windows reads nothing both when nothing has come
                // and when the program has gone. Writing nothing fails only
                // when it has gone.
                Ok(0) if cfg!(windows) && self.stream.write(&[]).is_ok() => return Ok(None),
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => self
                    .got
                    .extend_from_slice(chunk.get(..n).unwrap_or_default()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Send `line` and a line break, giving up when `gone` says to, so a
    /// program that stops reading cannot hold Lumit's side.
    fn send(&mut self, line: &str, gone: &dyn Fn() -> bool) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        let mut at = 0;
        while let Some(rest) = bytes.get(at..).filter(|rest| !rest.is_empty()) {
            if gone() {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            match self.stream.write(rest) {
                Ok(0) => std::thread::sleep(Duration::from_millis(1)),
                Ok(n) => at += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Why a program could not reach Lumit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreached {
    /// Lumit is not running, or the setting is off.
    Off,
    /// Another program is connected.
    Busy,
    /// Lumit let go without answering, as it does a wrong secret.
    Denied,
    /// What answered could not prove it is Lumit.
    NotLumit,
}

/// A program's connection to Lumit, once both ends have proved themselves.
pub struct Link {
    /// Lumit's lines.
    pub from: BufReader<RecvHalf>,
    /// Lines for Lumit.
    pub to: SendHalf,
}

/// The program's side: read `file`, connect, and prove it holds the secret.
///
/// # Errors
///
/// [`Unreached`] says why.
pub fn connect(file: &Path) -> Result<Link, Unreached> {
    let text = std::fs::read_to_string(file).map_err(|_| Unreached::Off)?;
    let (name, secret) = text.split_once('\n').ok_or(Unreached::Off)?;
    let secret = Secret::read_from(secret.as_bytes()).map_err(|_| Unreached::Off)?;
    let stream = if cfg!(windows) {
        name.to_ns_name::<GenericNamespaced>()
            .and_then(Stream::connect)
    } else {
        name.to_fs_name::<GenericFilePath>()
            .and_then(Stream::connect)
    }
    .map_err(|_| Unreached::Off)?;
    let (from, mut to) = stream.split();
    let mut from = BufReader::new(from);
    let opening: Opening = match read_json(&mut from) {
        Some(Ok(opening)) => opening,
        Some(Err(line)) if serde_json::from_str::<Refused>(&line).is_ok() => {
            return Err(Unreached::Busy)
        }
        _ => return Err(Unreached::NotLumit),
    };
    let ours = Nonce::generate().map_err(|_| Unreached::Off)?;
    let hello = Hello {
        nonce: ours,
        proof: Proof::broker(&secret, opening.nonce),
    };
    let hello = json_line(&hello).map_err(|_| Unreached::Off)?;
    writeln!(to, "{hello}").map_err(|_| Unreached::Denied)?;
    match read_json::<Answer>(&mut from) {
        None => Err(Unreached::Denied),
        Some(Ok(answer)) if answer.proof.matches(&Proof::host(&secret, ours)) => {
            Ok(Link { from, to })
        }
        Some(_) => Err(Unreached::NotLumit),
    }
}

/// The next line read as `T`, the line itself when it is something else, or
/// `None` when the other end has gone.
fn read_json<T: for<'de> Deserialize<'de>>(
    from: &mut BufReader<RecvHalf>,
) -> Option<Result<T, String>> {
    let mut line = String::new();
    match from.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(serde_json::from_str(&line).map_err(|_| line)),
    }
}
