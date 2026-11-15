//! An export run in a `lumit-cli` of its own: the queue's way of doing a
//! remote export another computer sent here.
//!
//! The open project's analyses, its simulation bakes and its sound belong to
//! this process, and a job is somebody else's project, so it runs in another.
//! A crash in it then ends the job and nothing else. It says how far it has
//! got one line at a time on its output (`cli::job`), and listens for
//! `pause`, `go` and `stop` on its input.
//!
//! Threads: one reads its output and one its notes, each writing to the job's
//! log as it goes. The queue reads what they heard when it is polled.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a child told to stop has before it is ended for it.
const GRACE: Duration = Duration::from_secs(5);

/// How long the last line of a child that has gone is looked for.
const LAST_LINE: Duration = Duration::from_millis(500);

/// What the child said.
pub(super) enum Heard {
    Progress {
        frame: usize,
        total: usize,
        encoder: Option<String>,
    },
    Done,
    Failed(String),
}

/// A child export, while it runs.
pub(super) struct Child {
    process: std::process::Child,
    input: Option<ChildStdin>,
    heard: Receiver<Heard>,
    /// When it was told to stop, so one that does not is ended.
    stopping: Option<Instant>,
    /// When it was first seen to have gone.
    gone: Option<Instant>,
    /// Whether it has said how it ended.
    ended: bool,
}

/// Where `lumit-cli` is: `LUMIT_CLI` when it is set, which tests use, and
/// otherwise where the packaging puts it. That is `lib\` below Lumit on
/// Windows and beside it elsewhere, and in a build the target folder, which
/// is one up from a test's own.
pub(crate) fn program() -> Option<PathBuf> {
    if let Some(set) = std::env::var_os("LUMIT_CLI") {
        return Some(PathBuf::from(set));
    }
    let name = if cfg!(windows) {
        "lumit-cli.exe"
    } else {
        "lumit-cli"
    };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let places = [
        Some(dir.join("lib").join(name)),
        Some(dir.join(name)),
        dir.parent().map(|up| up.join(name)),
    ];
    places.into_iter().flatten().find(|place| place.is_file())
}

/// Start the export the job folder `folder` holds.
pub(super) fn start(folder: &Path) -> Result<Child, String> {
    let program = program().ok_or("export: lumit-cli was not found beside Lumit")?;
    let log = std::fs::File::create(folder.join("log.txt"))
        .map_err(|e| format!("export: the job's log could not be written: {e}"))?;
    let log = Arc::new(Mutex::new(log));
    let mut command = Command::new(program);
    command
        .arg("export-job")
        .arg(folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // No console window flashing up for it.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut process = command
        .spawn()
        .map_err(|e| format!("export: lumit-cli would not start: {e}"))?;
    // Room for a few seconds of lines. Progress that finds no room is
    // dropped, as more is coming. The end always waits for a place.
    let (tell, heard) = sync_channel(64);
    if let Some(out) = process.stdout.take() {
        let (log, tell) = (log.clone(), tell.clone());
        let _ = std::thread::Builder::new()
            .name("lumit-export-child".into())
            .spawn(move || read_output(out, &log, &tell));
    }
    if let Some(notes) = process.stderr.take() {
        let _ = std::thread::Builder::new()
            .name("lumit-export-child-notes".into())
            .spawn(move || {
                for line in BufReader::new(notes).lines().map_while(Result::ok) {
                    write_log(&log, &line);
                }
            });
    }
    Ok(Child {
        input: process.stdin.take(),
        process,
        heard,
        stopping: None,
        gone: None,
        ended: false,
    })
}

fn write_log(log: &Mutex<std::fs::File>, line: &str) {
    if let Ok(mut log) = log.lock() {
        let _ = writeln!(log, "{line}");
    }
}

fn read_output(
    out: std::process::ChildStdout,
    log: &Mutex<std::fs::File>,
    tell: &SyncSender<Heard>,
) {
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        write_log(log, &line);
        let Ok(said) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let number = |name: &str| said.get(name).and_then(serde_json::Value::as_u64);
        if let Some(frame) = number("frame") {
            let progress = Heard::Progress {
                frame: usize::try_from(frame).unwrap_or(usize::MAX),
                total: usize::try_from(number("total").unwrap_or(0)).unwrap_or(usize::MAX),
                encoder: said
                    .get("encoder")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            };
            let _ = tell.try_send(progress);
        } else if said.get("done").is_some() {
            let _ = tell.send(Heard::Done);
        } else if let Some(why) = said.get("failed").and_then(serde_json::Value::as_str) {
            let _ = tell.send(Heard::Failed(why.to_owned()));
        }
    }
}

impl Child {
    /// Say `line` to the child. One that has gone hears nothing.
    fn say(&mut self, line: &str) {
        if let Some(input) = self.input.as_mut() {
            if writeln!(input, "{line}")
                .and_then(|()| input.flush())
                .is_err()
            {
                self.input = None;
            }
        }
    }

    pub(super) fn hold(&mut self, on: bool) {
        self.say(if on { "pause" } else { "go" });
    }

    pub(super) fn stop(&mut self) {
        self.say("stop");
        self.stopping.get_or_insert_with(Instant::now);
    }

    /// What it has said since it was last asked, and once it has gone, how
    /// it ended if it never said.
    pub(super) fn heard(&mut self) -> Vec<Heard> {
        let mut heard: Vec<Heard> = self.heard.try_iter().collect();
        self.ended |= said_end(&heard);
        if self.stopping.is_some_and(|at| at.elapsed() >= GRACE) {
            let _ = self.process.kill();
        }
        if self.ended {
            return heard;
        }
        if let Ok(Some(status)) = self.process.try_wait() {
            // Its output may still hold its last line, which the reader
            // passes on before it goes. Looked for again on the next poll,
            // never waited for here, as the queue is locked while it asks.
            let gone = *self.gone.get_or_insert_with(Instant::now);
            loop {
                match self.heard.try_recv() {
                    Ok(last) => heard.push(last),
                    Err(TryRecvError::Empty) if gone.elapsed() < LAST_LINE => {
                        self.ended = said_end(&heard);
                        return heard;
                    }
                    Err(_) => break,
                }
            }
            self.ended = said_end(&heard);
            if !self.ended {
                self.ended = true;
                heard.push(Heard::Failed(if self.stopping.is_some() {
                    "cancelled".to_owned()
                } else {
                    format!("export: lumit-cli stopped ({status})")
                }));
            }
        }
        heard
    }
}

/// Whether a child said how it ended.
fn said_end(heard: &[Heard]) -> bool {
    heard
        .iter()
        .any(|h| matches!(h, Heard::Done | Heard::Failed(_)))
}

impl Drop for Child {
    fn drop(&mut self) {
        // A child still running when the queue lets go of it is ended.
        if !matches!(self.process.try_wait(), Ok(Some(_))) {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }
}
