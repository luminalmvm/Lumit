//! `lumit-cli`, Lumit without its window.
//!
//! # In plain terms
//!
//! Three jobs from a terminal or another program: export a comp of a project
//! (`export`), run a script on a project, saving it after if asked (`run`),
//! and let a program that speaks MCP edit a project through the commands
//! (`mcp`), or the project open in Lumit when the person allows it. All
//! open the project and export the way Lumit does, through
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
mod relay;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use host::Headless;
use lumit_core::time::CompTime;
use lumit_render::export::ExportSpec;
use lumit_script::{ExportState, Host, Limits, RunOptions};
use uuid::Uuid;

use crate::api::export::BridgeExportSpec;

const HELP: &str = "\
Lumit on the command line.

Usage:
  lumit-cli export <project.lum> [--comp <name>]... [--all-comps] [--preset <name>]
                   [--range <in>-<out>] [--out <file>]
  lumit-cli run <script.js> [--project <project.lum>] [--save | --save-as <file>]
                [--arg <key>=<value>]...
  lumit-cli mcp [--project <project.lum>]
  lumit-cli --help
  lumit-cli --version

export     Export comps of a project, by default as an H.264 mp4 with sound.
  --comp       The comp, by name or id. Needed when the project has more than one.
               Repeat it for more comps, each to its own file.
  --all-comps  Every comp in the project.
  --preset     A saved export preset, by the name Lumit's Export window shows.
  --range      The frames to export, first and last, such as 0-119. Without it,
               the work area, or the whole comp when there is none.
  --out        The file to write. With several comps, the folder they go in.
               Without it, beside the project, named as Lumit's Export window
               names them, never over a file.

run        Run a JavaScript or After Effects (.jsx) script on a project, or on a
           new empty one.
  --project  The project to open.
  --save     Save the project over its file when the script has finished.
  --save-as  Save it as this file instead.
  --arg      Text for the script, read as lumit.args. A file named this way
             can be read by the script.

mcp        Answer MCP on standard input and output, so another program can edit
           a project through Lumit's commands. Without --project, the project
           open in Lumit, when Let other programs drive Lumit is on in Settings.
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
        how: Export,
    },
    Run {
        script: PathBuf,
        project: Option<PathBuf>,
        save: Save,
        args: BTreeMap<String, String>,
    },
    Mcp {
        /// `None` for the project open in Lumit.
        project: Option<PathBuf>,
    },
}

/// Which comps `export` writes, and how.
#[derive(Debug, Default, PartialEq)]
struct Export {
    /// By name or id. None and not `all` is the project's only comp.
    comps: Vec<String>,
    all: bool,
    preset: Option<String>,
    /// First and last frame, both exported.
    range: Option<(u64, u64)>,
    /// The file, or with several comps the folder.
    out: Option<PathBuf>,
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
        Command::Export { project, how } => export(&project, how),
        Command::Run {
            script,
            project,
            save,
            args,
        } => run(&script, project.as_deref(), save, args),
        Command::Mcp { project: None } => relay::relay(),
        Command::Mcp {
            project: Some(project),
        } => mcp(&project),
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
    let mut how = Export::default();
    let mut project = None;
    let mut save = Save::No;
    let mut script_args = BTreeMap::new();
    rest.reverse();
    while let Some(arg) = rest.pop() {
        let flag = arg.to_str().unwrap_or_default().to_owned();
        match (first.as_str(), flag.as_str()) {
            ("export", "--comp") => how.comps.push(text(wants(&flag, rest.pop())?)?),
            ("export", "--all-comps") => how.all = true,
            ("export", "--preset") => how.preset = Some(text(wants(&flag, rest.pop())?)?),
            ("export", "--range") => {
                let range = text(wants(&flag, rest.pop())?)?;
                let frames = range
                    .split_once('-')
                    .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
                    .filter(|(a, b)| a <= b)
                    .ok_or_else(|| {
                        format!("--range takes a first and last frame, such as 0-119, not {range}.")
                    })?;
                how.range = Some(frames);
            }
            ("export", "--out") => how.out = Some(PathBuf::from(wants(&flag, rest.pop())?)),
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
        "export" => {
            if how.all && !how.comps.is_empty() {
                return Err("--all-comps and --comp go one at a time.".into());
            }
            Ok(Command::Export {
                project: one("a project")?,
                how,
            })
        }
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

/// Files of image sequences that would not read, each drawn as a neighbour.
fn note_unreadable_frames() {
    #[cfg(feature = "media")]
    {
        let files: Vec<String> = lumit_media::sequence::unreadable_frames()
            .into_iter()
            .map(|(_, file)| {
                // Footage found from the project's folder has its path run
                // through it, so each .. is walked back for a plain line.
                let plain = file.components().fold(PathBuf::new(), |mut plain, part| {
                    if part == std::path::Component::ParentDir {
                        plain.pop();
                    } else {
                        plain.push(part);
                    }
                    plain
                });
                plain.display().to_string()
            })
            .collect();
        if !files.is_empty() {
            note!("Frames that could not be read: {}", files.join(", "));
        }
    }
}

fn export(project: &Path, how: Export) -> Result<(), String> {
    let stop = interrupt::stop_flag();
    // The preset first, so a misspelt name is heard before the project opens.
    let settings = match &how.preset {
        Some(name) => crate::export::preset_get(name).ok_or_else(|| {
            let names: Vec<String> = crate::export::preset_list()
                .into_iter()
                .map(|p| p.name)
                .collect();
            format!(
                "There is no export preset called \"{name}\". The presets are: {}.",
                names.join(", ")
            )
        })?,
        None => crate::export::from_export_spec(&ExportSpec::default())
            .ok_or("This build of Lumit cannot export.")?,
    };
    let host = open(project)?;
    let comps = if how.all {
        host.comps()
    } else if how.comps.is_empty() {
        vec![host.find_comp(None)?]
    } else {
        how.comps
            .iter()
            .map(|key| host.find_comp(Some(key)))
            .collect::<Result<_, _>>()?
    };
    if comps.is_empty() {
        return Err("The project has no comp.".into());
    }
    let beside = host
        .path()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    // With several comps, --out is where they all go.
    let folder = match &how.out {
        Some(out) if comps.len() > 1 => {
            if !out.is_dir() {
                return Err(format!(
                    "With several comps, --out is the folder they go in, and {} is not one.",
                    out.display()
                ));
            }
            std::path::absolute(out).unwrap_or_else(|_| out.clone())
        }
        _ => beside,
    };
    let mut failed = 0;
    for (comp, name) in &comps {
        if stop.load(Ordering::Relaxed) {
            return Err("Export stopped.".into());
        }
        let file = match &how.out {
            Some(out) if comps.len() == 1 => {
                let out = std::path::absolute(out).unwrap_or_else(|_| out.clone());
                PathBuf::from(crate::export::with_format_extension(
                    &out.to_string_lossy(),
                    &settings.codec,
                ))
            }
            _ => export_name(&folder, &settings, name),
        };
        match export_one(&host, &stop, *comp, &settings, how.range, file) {
            Ok(path) => out!("{}", path.display()),
            // One comp's failure is the program's.
            Err(why) if comps.len() == 1 => return Err(why),
            Err(why) => {
                note!("{name}: {why}");
                failed += 1;
            }
        }
    }
    note_unreadable_frames();
    match failed {
        0 => Ok(()),
        n => Err(format!("{n} of {} exports failed.", comps.len())),
    }
}

/// Where comp `name` exports to in `dir` when nobody says: named as the
/// Export window names it, from the file name set in Settings > Export or
/// else the comp's name, and never over a file already there.
fn export_name(dir: &Path, settings: &BridgeExportSpec, name: &str) -> PathBuf {
    let template = crate::export::defaults_get().filename_template;
    let template = if template.trim().is_empty() {
        "{comp}".to_owned()
    } else {
        template
    };
    let file = crate::api::export::export_preset(settings.preset.clone(), name.into(), template)
        .default_name;
    let stem = file.strip_suffix(".mp4").unwrap_or(&file);
    let path = dir.join(stem).to_string_lossy().into_owned();
    let path = crate::export::with_format_extension(&path, &settings.codec);
    if crate::export::path_taken(&path, &settings.codec) {
        PathBuf::from(crate::export::free_path(&path, &settings.codec))
    } else {
        PathBuf::from(path)
    }
}

/// Export `comp` to `file` with `settings` over frames `range`, and wait for
/// it. Answers the file written.
fn export_one(
    host: &Headless,
    stop: &AtomicBool,
    comp: Uuid,
    settings: &BridgeExportSpec,
    range: Option<(u64, u64)>,
    file: PathBuf,
) -> Result<PathBuf, String> {
    let doc = host.store().snapshot();
    let found = doc.comp(comp).ok_or("The comp has gone.")?;
    let mut settings = settings.clone();
    if let Some((first, last)) = range {
        let frames = found.frame_rate.frame_at(CompTime(found.duration.0));
        if first >= u64::try_from(frames).unwrap_or(0) {
            return Err(format!("The comp has {frames} frames."));
        }
        settings.range_start_frame = i64::try_from(first).unwrap_or(i64::MAX);
        // The exporter's range stops short of its end.
        settings.range_end_frame = i64::try_from(last).map_or(i64::MAX, |l| l.saturating_add(1));
    }
    let spec = crate::export::to_export_spec(&settings, found.width, found.height)?;
    note!("Exporting {} to {}", found.name, file.display());
    let job = host
        .start_export(comp, file, spec)
        .map_err(|e| e.to_string())?;
    wait_for(host, job, stop)
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
            // No window here, so a script's dialog is cancelled.
            dialog: None,
            // The person running this is the one aiming it.
            limits: Limits::Off,
        },
    );
    for line in &report.log {
        note!("{line}");
    }
    let finished = finish_exports(&host, &stop, report.result.is_ok());
    note_unreadable_frames();
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
