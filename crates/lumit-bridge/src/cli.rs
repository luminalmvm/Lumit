//! `lumit-cli`, Lumit without its window.
//!
//! # In plain terms
//!
//! Three jobs from a terminal or another program: export a comp of a project
//! (`export`), run a script on a project, saving it after if asked (`run`),
//! and let a program that speaks MCP edit a project through the commands
//! (`mcp`). All open the project and export the way Lumit does, through
//! [`host::Headless`]. What happens is told on standard error, and what
//! comes out, the exported file or the script's answer, on standard output.
//!
//! The program itself, `crates/lumit-cli`, is a small launcher. It loads this
//! library from beside it and calls [`lumit_cli_main`], so the command line
//! runs the engine the app ships rather than a second copy of it.
//!
//! Exit status: 0 done, 1 failed, 2 the arguments were wrong.

/// A line on standard output. A reader that has gone, as `| head` leaves
/// it, ends the program there rather than in a panic.
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::cli::write_out(format_args!($($arg)*))
    };
}

mod host;
mod interrupt;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use host::Headless;
use lumit_script::{ExportState, Host, Limits, RunOptions};

const HELP: &str = "\
Lumit on the command line.

Usage:
  lumit-cli export <project.lum> [--comp <name>] [--out <file>]
  lumit-cli run <script.js> [--project <project.lum>] [--save | --save-as <file>]
                [--arg <key>=<value>]...
  lumit-cli mcp --project <project.lum>
  lumit-cli --help
  lumit-cli --version

export     Export a comp of a project as an H.264 mp4 with sound.
  --comp     The comp, by name or id. Needed when the project has more than one.
  --out      The file to write. Without it, the comp's name beside the project.

run        Run a JavaScript or After Effects (.jsx) script on a project, or on a
           new empty one.
  --project  The project to open.
  --save     Save the project over its file when the script has finished.
  --save-as  Save it as this file instead.
  --arg      Text for the script, read as lumit.args. A file named this way
             can be read by the script.

mcp        Answer MCP on standard input and output, so another program can edit
           a project through Lumit's commands.
  --project  The project. Footage comes from its folder, exports go into it,
             and it saves only when asked.

Exit status: 0 done, 1 failed, 2 the arguments were wrong.
";

/// What was asked for.
#[derive(Debug, PartialEq)]
enum Command {
    Help,
    Version,
    Export {
        project: PathBuf,
        comp: Option<String>,
        out: Option<PathBuf>,
    },
    Run {
        script: PathBuf,
        project: Option<PathBuf>,
        save: Save,
        args: BTreeMap<String, String>,
    },
    Mcp {
        project: PathBuf,
    },
}

#[derive(Debug, PartialEq)]
enum Save {
    No,
    Over,
    As(PathBuf),
}

/// The command line, as the `lumit-cli` launcher calls it. It reads the
/// process's own arguments and answers the exit status.
#[no_mangle]
pub extern "C" fn lumit_cli_main() -> i32 {
    // A panic has already been told on standard error, and is a failure
    // like any other. It must not unwind into the launcher.
    std::panic::catch_unwind(main).unwrap_or(1)
}

fn main() -> i32 {
    find_brokers();
    let command = match parse(std::env::args_os().skip(1).collect()) {
        Ok(command) => command,
        Err(why) => {
            note!("lumit-cli: {why}\nTry lumit-cli --help.");
            return 2;
        }
    };
    let done = match command {
        Command::Help => {
            out!("{}", HELP.trim_end());
            Ok(())
        }
        Command::Version => {
            out!("lumit-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Export { project, comp, out } => export(&project, comp.as_deref(), out),
        Command::Run {
            script,
            project,
            save,
            args,
        } => run(&script, project.as_deref(), save, args),
        Command::Mcp { project } => mcp(&project),
    };
    match done {
        Ok(()) => 0,
        Err(why) => {
            note!("{why}");
            1
        }
    }
}

/// Write `line` and a line break to standard output, and end the program
/// when nobody is reading it.
fn write_out(line: std::fmt::Arguments<'_>) {
    let mut output = std::io::stdout().lock();
    if let Err(e) = writeln!(output, "{line}").and_then(|()| output.flush()) {
        let code = if e.kind() == std::io::ErrorKind::BrokenPipe {
            0
        } else {
            1
        };
        std::process::exit(code);
    }
}

/// Each broker is looked for beside the program that starts it. On Windows
/// this one is in lib\, a folder below Lumit and its brokers, so it says
/// where they are. Before any thread starts, as setting a variable must be.
fn find_brokers() {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return;
    };
    #[cfg(feature = "media")]
    let media = if cfg!(windows) {
        "lumit-media-broker.exe"
    } else {
        "lumit-media-broker"
    };
    let brokers = [
        (
            lumit_ofx::ipc::broker::BROKER_EXE_ENV,
            lumit_ofx::ipc::broker::broker_exe_name(),
        ),
        (
            lumit_aplug::ipc::broker::BROKER_EXE_ENV,
            lumit_aplug::ipc::broker::broker_exe_name(),
        ),
        #[cfg(feature = "media")]
        (lumit_media::ai::BROKER_EXE_ENV, media),
    ];
    for (variable, name) in brokers {
        if std::env::var_os(variable).is_some() || dir.join(name).is_file() {
            continue;
        }
        if let Some(up) = dir.parent().map(|up| up.join(name)).filter(|p| p.is_file()) {
            std::env::set_var(variable, up);
        }
    }
}

fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Command::Help);
    };
    let text = |arg: OsString| {
        arg.into_string()
            .map_err(|arg| format!("{} is not text this can read.", arg.to_string_lossy()))
    };
    let first = text(first)?;
    let mut rest: Vec<OsString> = args.collect();
    // --help anywhere is a request for help.
    if matches!(first.as_str(), "--help" | "-h" | "help")
        || rest.iter().any(|a| a == "--help" || a == "-h")
    {
        return Ok(Command::Help);
    }
    if matches!(first.as_str(), "--version" | "-V") {
        return Ok(Command::Version);
    }
    let wants = |flag: &str, value: Option<OsString>| {
        value.ok_or_else(|| format!("{flag} needs a value after it."))
    };
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut comp = None;
    let mut out = None;
    let mut project = None;
    let mut save = Save::No;
    let mut script_args = BTreeMap::new();
    rest.reverse();
    while let Some(arg) = rest.pop() {
        let flag = arg.to_str().unwrap_or_default().to_owned();
        match (first.as_str(), flag.as_str()) {
            ("export", "--comp") => comp = Some(text(wants(&flag, rest.pop())?)?),
            ("export", "--out") => out = Some(PathBuf::from(wants(&flag, rest.pop())?)),
            ("run" | "mcp", "--project") => {
                project = Some(PathBuf::from(wants(&flag, rest.pop())?))
            }
            ("run", "--save") | ("run", "--save-as") if save != Save::No => {
                return Err("--save and --save-as go one at a time.".into());
            }
            ("run", "--save") => save = Save::Over,
            ("run", "--save-as") => save = Save::As(PathBuf::from(wants(&flag, rest.pop())?)),
            ("run", "--arg") => {
                let pair = text(wants(&flag, rest.pop())?)?;
                let (key, value) = pair
                    .split_once('=')
                    .filter(|(key, _)| !key.is_empty())
                    .ok_or_else(|| format!("--arg takes key=value, not {pair}."))?;
                script_args.insert(key.to_owned(), value.to_owned());
            }
            (_, flag) if flag.starts_with("--") => {
                return Err(format!("{first} has no option {flag}."));
            }
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let one = |what: &str| match positional.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("{first} needs {what}.")),
        _ => Err(format!("{first} takes one {what}.")),
    };
    match first.as_str() {
        "export" => Ok(Command::Export {
            project: one("a project")?,
            comp,
            out,
        }),
        "run" => {
            if save == Save::Over && project.is_none() {
                return Err("--save needs --project. A new project saves with --save-as.".into());
            }
            Ok(Command::Run {
                script: one("a script")?,
                project,
                save,
                args: script_args,
            })
        }
        "mcp" => {
            if !positional.is_empty() {
                return Err("mcp takes the project with --project.".into());
            }
            // Driving the Lumit that is open is still to come.
            let project = project
                .ok_or("mcp needs --project. Connecting to a running Lumit is not built yet.")?;
            Ok(Command::Mcp { project })
        }
        other => Err(format!("There is no command called {other}.")),
    }
}

/// Open the project at `path`, saying which footage is missing.
fn open(path: &Path) -> Result<Headless, String> {
    note!("Opening {}", path.display());
    let (host, missing) = Headless::open(path)?;
    if !missing.is_empty() {
        note!("Missing footage: {}", missing.join(", "));
    }
    Ok(host)
}

fn export(project: &Path, comp: Option<&str>, out: Option<PathBuf>) -> Result<(), String> {
    let stop = interrupt::stop_flag();
    let host = open(project)?;
    let (comp, name) = host.find_comp(comp)?;
    let job = host
        .export_start(comp, out.as_deref())
        .map_err(|e| e.to_string())?;
    let path = host.export_status(job).map(|s| s.path).unwrap_or_default();
    note!("Exporting {name} to {}", path.display());
    let path = wait_for(&host, job, &stop)?;
    out!("{}", path.display());
    Ok(())
}

/// Wait for export `job` to finish, showing how far it has got. Ctrl+C
/// stops it. Answers the file written.
fn wait_for(host: &Headless, job: u64, stop: &AtomicBool) -> Result<PathBuf, String> {
    let terminal = std::io::stderr().is_terminal();
    let mut shown = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = host.export_cancel(job);
        }
        let status = host.export_status(job).map_err(|e| e.to_string())?;
        let percent = (status.done * 100).checked_div(status.total).unwrap_or(0);
        // A whole line each tenth when nobody is watching it change.
        let step = if terminal { percent } else { percent / 10 * 10 };
        if status.state == ExportState::Running && shown != Some(step) {
            shown = Some(step);
            if terminal {
                let _ = write!(std::io::stderr(), "\r{percent}%  ");
            } else {
                note!("{percent}%");
            }
        }
        if status.state != ExportState::Running && terminal && shown.is_some() {
            note!("");
        }
        match status.state {
            ExportState::Running => {}
            ExportState::Done => return Ok(status.path),
            ExportState::Cancelled => return Err("Export stopped.".into()),
            ExportState::Failed => {
                let why = status.message.unwrap_or_default();
                return Err(format!("The export failed: {why}"));
            }
        }
    }
}

fn run(
    script: &Path,
    project: Option<&Path>,
    save: Save,
    args: BTreeMap<String, String>,
) -> Result<(), String> {
    let stop = interrupt::stop_flag();
    let source = lumit_ingress::read_to_string_capped(script, crate::script::MAX_SOURCE_BYTES)
        .map_err(|e| format!("{} could not be read: {e}", script.display()))?;
    let host = match project {
        Some(path) => open(path)?,
        None => Headless::new_project(),
    };
    // Claimed before the script runs, so a file Lumit has open is found out
    // before any work is done, and Lumit does not open it meanwhile.
    let target = match &save {
        Save::No => None,
        Save::Over => Some(host.save_target(None)),
        Save::As(path) => Some(host.save_target(Some(path))),
    };
    let target = target.transpose().map_err(|e| e.to_string())?;
    if let Some(target) = &target {
        host.claim(target).map_err(|e| e.to_string())?;
    }
    // Files named on the command line are the ones the script may read.
    let readable = args
        .values()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    let name = script
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Script".into());
    note!("Running {name}");
    let alert = |message: &str| note!("{message}");
    let report = lumit_script::run(
        &host,
        &source,
        RunOptions {
            name: name.clone(),
            stop: stop.clone(),
            readable,
            args,
            // Nobody to ask. Files come in with --arg.
            choose_file: &|_, _| None,
            // Absolute, so a bare file name still gives the script its folder.
            path: std::path::absolute(script).ok(),
            alert: Some(&alert),
            // The person running this is the one aiming it.
            limits: Limits::Off,
        },
    );
    for line in &report.log {
        note!("{line}");
    }
    let finished = finish_exports(&host, &stop, report.result.is_ok());
    let value = report.result.map_err(|e| format!("{name}: {e}"))?;
    finished?;
    if let Some(target) = target {
        let saved = host.save(Some(&target)).map_err(|e| e.to_string())?;
        note!("Saved {}", saved.display());
    }
    if !value.is_null() {
        let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        out!("{text}");
    }
    Ok(())
}

/// Answer MCP, one message a line on standard input and one reply a line on
/// standard output, until the client closes its end.
fn mcp(project: &Path) -> Result<(), String> {
    let host = open(project)?;
    // Held while the client is there, so Lumit does not open a project the
    // client may still save.
    let own = host.save_target(None).map_err(|e| e.to_string())?;
    host.claim(&own).map_err(|_| {
        format!(
            "{} is open in Lumit, or in another lumit-cli. Close it there first.",
            own.display()
        )
    })?;
    note!("Ready for MCP on standard input");
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = input
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("Standard input could not be read: {e}."))?;
        if read == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&line);
        // PowerShell puts a byte order mark before what it pipes in.
        let text = text.trim_start_matches('\u{feff}').trim();
        if text.is_empty() {
            continue;
        }
        if let Some(reply) = lumit_script::mcp::answer(text, &host) {
            writeln!(output, "{reply}")
                .and_then(|()| output.flush())
                .map_err(|e| format!("Standard output could not be written: {e}."))?;
        }
    }
    // Nobody is left to ask after an export, so it stops with the client.
    finish_exports(&host, &AtomicBool::new(false), false)
}

/// Exports a script started and did not wait for: let them finish, or stop
/// them when the script failed or Ctrl+C was pressed.
fn finish_exports(host: &Headless, stop: &AtomicBool, keep: bool) -> Result<(), String> {
    if !keep {
        host.cancel_exports();
    } else if host.exporting() {
        note!("Waiting for the export to finish");
    }
    while host.exporting() {
        if stop.load(Ordering::Relaxed) {
            host.cancel_exports();
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if keep && stop.load(Ordering::Relaxed) {
        return Err("Stopped.".into());
    }
    Ok(())
}
