//! Reading an Illustrator document in a process of its own.
//!
//! # In plain terms
//!
//! A file can state a small artboard and hold a picture that unpacks to more
//! memory than the machine has. How much it really needs is only known part
//! way through reading it, so no check beforehand catches it. The reading is
//! done in `lumit-media-broker` instead: one short-lived process a call, with
//! a cap on its memory and a limit on its time. A file that breaks the reader
//! ends the helper, and the caller gets an error.
//!
//! Both halves are here. [`serve`] is all the helper does, and [`open`] and
//! [`read_layer`] are the caller's side. The answer comes back on the helper's
//! standard output, and the caller checks its size before taking it.
//!
//! Runs on whichever thread asked for the file, and blocks it until the helper
//! answers or its time is up.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::time::Duration;

use lumit_ingress::{checked_raster_bytes, checked_usize, Budget};

use super::{bad, open_here, read_layer_here, AiDocument, LIMITS, MAX_SIDE, REASONS, UNREADABLE};
use crate::decode::{DecodedFrame, PixelFormat};
use crate::MediaError;

/// The environment variable that overrides where the helper is, for a test or
/// for running from a build tree.
pub const BROKER_EXE_ENV: &str = "LUMIT_MEDIA_BROKER";

/// How long the helper has to answer before it is stopped. A heavy document
/// draws in a few seconds, so this only ends a read that was never going to
/// finish.
pub(super) const DEADLINE: Duration = Duration::from_secs(60);

/// The most memory the helper may hold. A large document needs the file twice
/// while one layer is picked out and the frame twice while it is drawn, and
/// this leaves room for both. A file that asks for more is one that would have
/// taken the machine's memory.
const MEMORY_CAP: u64 = 4 << 30;

/// The longest layer list the caller takes, in bytes. Names are short, and
/// there are never more layers than `LIMITS` counts.
const MAX_LIST: u32 = 1 << 20;

/// Where the helper is: beside Lumit's own executable, which is where every
/// packaging step puts it.
fn broker_exe() -> Option<PathBuf> {
    if let Some(given) = std::env::var_os(BROKER_EXE_ENV) {
        return Some(PathBuf::from(given));
    }
    let name = if cfg!(windows) {
        "lumit-media-broker.exe"
    } else {
        "lumit-media-broker"
    };
    let exe = std::env::current_exe().ok()?;
    // A test program runs from a folder below the one Cargo builds the helper
    // into, so a debug build looks one folder up as well.
    let folders = if cfg!(debug_assertions) { 2 } else { 1 };
    for folder in exe.ancestors().skip(1).take(folders) {
        let helper = folder.join(name);
        if helper.is_file() {
            return Some(helper);
        }
    }
    None
}

/// A read with no helper to ask. A debug build reads in this process, so the
/// tests of the crates above pass without the helper built. A release build
/// refuses.
fn without_helper<T>(here: impl FnOnce() -> Result<T, MediaError>) -> Result<T, MediaError> {
    if cfg!(debug_assertions) {
        here()
    } else {
        Err(bad(
            "the helper that reads Illustrator files is missing from Lumit's folder",
        ))
    }
}

pub(super) fn open(path: &Path) -> Result<AiDocument, MediaError> {
    let Some(exe) = broker_exe() else {
        return without_helper(|| open_here(path));
    };
    let args = [OsStr::new("ai-open"), path.as_os_str()];
    ask(&exe, &args, DEADLINE, read_document)
}

pub(super) fn read_layer(
    path: &Path,
    index: Option<u32>,
    target_width: Option<u32>,
    deadline: Duration,
) -> Result<DecodedFrame, MediaError> {
    let Some(exe) = broker_exe() else {
        return without_helper(|| read_layer_here(path, index, target_width));
    };
    let number = |n: Option<u32>| n.map_or_else(|| "-".to_owned(), |n| n.to_string());
    let (index, target_width) = (number(index), number(target_width));
    let args = [
        OsStr::new("ai-read"),
        path.as_os_str(),
        OsStr::new(&index),
        OsStr::new(&target_width),
    ];
    ask(&exe, &args, deadline, read_frame)
}

/// All the helper does: read what `args` ask for and write the answer to
/// `out`. The arguments are `ai-open <path>`, or `ai-read <path>` with a layer
/// index and a target width after it, each a number or `-` for none.
pub fn serve(args: &[OsString], out: &mut dyn Write) -> Result<(), MediaError> {
    cap_own_memory();
    let text = |at: usize| args.get(at).and_then(|arg| arg.to_str());
    let number = |at: usize| text(at).and_then(|digits| digits.parse::<u32>().ok());
    match (text(0), args.get(1)) {
        (Some("ai-open"), Some(path)) => {
            let doc = open_here(Path::new(path))?;
            let list = bincode::serialize(&doc).map_err(|_| bad(UNREADABLE))?;
            let length = u32::try_from(list.len()).map_err(|_| bad(UNREADABLE))?;
            out.write_all(&length.to_le_bytes())?;
            out.write_all(&list)?;
        }
        (Some("ai-read"), Some(path)) => {
            let frame = read_layer_here(Path::new(path), number(2), number(3))?;
            out.write_all(&frame.width.to_le_bytes())?;
            out.write_all(&frame.height.to_le_bytes())?;
            out.write_all(&frame.rgba)?;
        }
        _ => return Err(bad("the helper was not asked for anything it does")),
    }
    out.flush()?;
    Ok(())
}

/// Whether a side the helper states is one that is ever drawn.
fn side_in_range(side: u32) -> bool {
    (1..=MAX_SIDE).contains(&side)
}

/// The layer list off the helper's output: its length, then that many bytes.
fn read_document(from: &mut dyn Read) -> Option<AiDocument> {
    let mut length = [0u8; 4];
    from.read_exact(&mut length).ok()?;
    let length = u32::from_le_bytes(length);
    if length > MAX_LIST {
        return None;
    }
    let mut list = vec![0u8; usize::try_from(length).ok()?];
    from.read_exact(&mut list).ok()?;
    let doc: AiDocument = bincode::deserialize(&list).ok()?;
    (side_in_range(doc.width) && side_in_range(doc.height)).then_some(doc)
}

/// A frame off the helper's output: its width and height, then its pixels.
/// The size is the helper's word for it, so it is held to the same limits as
/// the file's own before anything is set aside for it.
fn read_frame(from: &mut dyn Read) -> Option<DecodedFrame> {
    let mut number = [0u8; 4];
    from.read_exact(&mut number).ok()?;
    let width = u32::from_le_bytes(number);
    from.read_exact(&mut number).ok()?;
    let height = u32::from_le_bytes(number);
    if !side_in_range(width) || !side_in_range(height) {
        return None;
    }
    let bytes = checked_raster_bytes(u64::from(width), u64::from(height), 4, 1).ok()?;
    let mut rgba = Budget::new(LIMITS)
        .vec_with_capacity::<u8>(checked_usize(bytes).ok()?)
        .ok()?;
    from.take(bytes).read_to_end(&mut rgba).ok()?;
    (u64::try_from(rgba.len()).ok() == Some(bytes)).then_some(DecodedFrame {
        width,
        height,
        rgba,
        format: PixelFormat::Srgb8,
    })
}

/// Start the helper with no console window of its own. It is a console
/// program and Lumit is a windowed one, so on Windows each one would open a
/// window in front of the editor.
fn no_console(command: &mut Command) {
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

/// Run the helper once and take its answer with `read`.
///
/// The helper is stopped whatever happens: it has answered, it has run past
/// `deadline`, or what it wrote was not an answer.
fn ask<T: Send + 'static>(
    exe: &Path,
    args: &[&OsStr],
    deadline: Duration,
    read: fn(&mut dyn Read) -> Option<T>,
) -> Result<T, MediaError> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_console(&mut command);
    let would_not_start = || bad("the helper that reads Illustrator files would not start");
    let mut child = command.spawn().map_err(|_| would_not_start())?;
    // Kept until the helper has gone. A helper that can't be capped is not
    // left to run.
    let cap = cap_memory(&child);
    let (Some(mut answer), Some(reason), Some(_)) =
        (child.stdout.take(), child.stderr.take(), &cap)
    else {
        stop(&mut child);
        return Err(would_not_start());
    };

    // The answer is read on a thread of its own so the wait for it can end.
    // Stopping the helper closes its output, which ends the thread.
    let (send, receive) = sync_channel(1);
    let reading = std::thread::Builder::new()
        .name("ai-helper".to_owned())
        .spawn(move || {
            let _ = send.send(read(&mut answer));
        });
    let answer = match reading {
        Ok(_) => receive.recv_timeout(deadline),
        Err(_) => Err(RecvTimeoutError::Disconnected),
    };
    stop(&mut child);
    match answer {
        Ok(Some(answer)) => Ok(answer),
        Err(RecvTimeoutError::Timeout) => Err(bad("the file took too long to read")),
        _ => Err(said(reason)),
    }
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The helper's reason for failing, off its error output, as one of this
/// module's own. A helper that died without giving one could not read the
/// file.
fn said(reason: impl Read) -> MediaError {
    let mut text = String::new();
    let _ = reason.take(1024).read_to_string(&mut text);
    let known = REASONS
        .iter()
        .find(|known| text.trim_end().ends_with(**known));
    bad(known.copied().unwrap_or(UNREADABLE))
}

// ---------------------------------------------------------------------------
// The memory cap
// ---------------------------------------------------------------------------

/// A Windows job object holding the helper to [`MEMORY_CAP`]. Closing it ends
/// the helper, so one is never left running after Lumit has gone.
#[cfg(windows)]
struct Job(*mut std::ffi::c_void);

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(
        attributes: *const std::ffi::c_void,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn SetInformationJobObject(
        job: *mut std::ffi::c_void,
        class: u32,
        information: *const std::ffi::c_void,
        length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: *mut std::ffi::c_void, process: *mut std::ffi::c_void) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

/// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`, from winnt.h.
#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct JobLimits {
    per_process_user_time: i64,
    per_job_user_time: i64,
    flags: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[cfg(all(windows, target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<JobLimits>() == 144);

#[cfg(windows)]
impl Drop for Job {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateJobObjectW`, was checked, and is
        // closed nowhere else.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Put the helper under the memory cap. `None` when it could not be done.
#[cfg(windows)]
#[allow(unsafe_code)]
fn cap_memory(child: &Child) -> Option<Job> {
    use std::os::windows::io::AsRawHandle;
    /// `JobObjectExtendedLimitInformation`, from winnt.h.
    const EXTENDED_LIMITS: u32 = 9;
    /// `JOB_OBJECT_LIMIT_PROCESS_MEMORY` and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    const FLAGS: u32 = 0x0100 | 0x2000;

    let limits = JobLimits {
        flags: FLAGS,
        process_memory: usize::try_from(MEMORY_CAP).unwrap_or(usize::MAX),
        ..JobLimits::default()
    };
    // SAFETY: plain Win32 calls. The job handle is checked before it is used
    // and `Job` closes it. `limits` has the layout the call reads and outlives
    // it, and the length passed is its own. The process handle is `child`'s,
    // which is open for as long as `child` is.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let job = Job(job);
        let set = SetInformationJobObject(
            job.0,
            EXTENDED_LIMITS,
            std::ptr::from_ref(&limits).cast(),
            u32::try_from(std::mem::size_of::<JobLimits>()).ok()?,
        );
        (set != 0 && AssignProcessToJobObject(job.0, child.as_raw_handle()) != 0).then_some(job)
    }
}

/// Off Windows the helper caps itself as it starts: see [`cap_own_memory`].
#[cfg(not(windows))]
fn cap_memory(_child: &Child) -> Option<()> {
    Some(())
}

/// The helper's cap on its own memory, set before it opens the file.
#[cfg(unix)]
#[allow(unsafe_code)]
fn cap_own_memory() {
    /// `struct rlimit`, from sys/resource.h.
    #[repr(C)]
    struct Limit {
        soft: u64,
        hard: u64,
    }
    extern "C" {
        fn setrlimit(resource: std::os::raw::c_int, limit: *const Limit) -> std::os::raw::c_int;
    }
    /// `RLIMIT_AS`, from sys/resource.h. macOS takes the call and does not
    /// hold a process to it, so there the deadline is the only limit.
    const ADDRESS_SPACE: std::os::raw::c_int = if cfg!(target_os = "macos") { 5 } else { 9 };

    let limit = Limit {
        soft: MEMORY_CAP,
        hard: MEMORY_CAP,
    };
    // SAFETY: a plain libc call that reads `limit`, which has the layout it
    // expects on every 64-bit target and outlives it.
    unsafe {
        setrlimit(ADDRESS_SPACE, &limit);
    }
}

/// On Windows the caller caps the helper: see [`cap_memory`].
#[cfg(not(unix))]
fn cap_own_memory() {}
