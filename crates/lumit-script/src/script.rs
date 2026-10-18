//! Running a JavaScript script against the open project.
//!
//! # In plain terms
//!
//! A script sees one object, `lumit`. `lumit.command("layer.add", { comp:
//! "Main", kind: "camera" })` runs any command, and every command is also a
//! method of its own: `lumit.layer.add({ ... })`. Besides those a script can
//! ask the person for a file (`lumit.chooseFile`), read a file in its own
//! folder or one it was given or that was chosen (`lumit.readText`,
//! `lumit.readJson`), and turn another program's seconds into a frame of a
//! comp (`lumit.frameAt`). `print` and `console.log` go to the run's log.
//!
//! The engine is QuickJS with none of its own file, network or process
//! modules, so a script reaches the project and the files above and nothing
//! else. The whole run is one undo step, taken back if the script fails or is
//! stopped.
//!
//! The [`Host`] stays on the caller's thread and the script runs on a thread
//! of its own, so Stop can end a script that loops for ever. Each `lumit` call
//! is handed back to the caller's thread to run, and the script waits for the
//! answer.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use lumit_core::time::{CompTime, FrameRate, Rational};
use rquickjs::context::EvalOptions;
use rquickjs::convert::Coerced;
use rquickjs::function::Opt;
use rquickjs::{CatchResultExt, CaughtError, Context, Ctx, Exception, FromJs, Function, Object};
use rquickjs::{Runtime, Value as JsValue};
use serde_json::{json, Value};

use crate::reach::{self, Reach};
use crate::{commands, one_step, Host, ScriptError, COMMANDS};

/// The `lumit` object, built in JavaScript over the runner's own functions.
const LUMIT_JS: &str = include_str!("lumit.js");
/// After Effects' scripting objects, built over `lumit`.
const AE_JS: &str = include_str!("ae.js");

/// The most memory a script's values may take. Stop is what ends a script
/// that runs away; this keeps one nobody is watching, on the command line,
/// from eating the machine.
const MAX_MEMORY: usize = 1 << 30;
/// The most a script may read from one file.
const MAX_READ_BYTES: u64 = 64 << 20;
/// The most entries a script is told of in one folder.
const MAX_LISTED: usize = 100_000;
/// Lines of `print` kept. A loop printing for ever stops being heard here.
const MAX_LOG_LINES: usize = 10_000;
/// The longest line printed or alert shown, and the most the whole log
/// holds, so a script cannot fill the interface or a reply with text.
const MAX_LINE_BYTES: usize = 4 << 10;
const MAX_LOG_BYTES: usize = 1 << 20;
/// The script thread's stack, and how much of it the engine may use, which
/// leaves room for the runner's own frames under a deeply recursive script.
const STACK_BYTES: usize = 16 << 20;
const ENGINE_STACK_BYTES: usize = 8 << 20;

/// What a run needs besides the host and the script.
pub struct RunOptions<'a> {
    /// The script's name, its file name for one read from a file.
    pub name: String,
    /// Set it and the script stops at its next step, as if it had failed.
    pub stop: Arc<AtomicBool>,
    /// Files the script may read without asking, such as those passed on the
    /// command line.
    pub readable: Vec<PathBuf>,
    /// Text the caller hands the script, read as `lumit.args`.
    pub args: BTreeMap<String, String>,
    /// Ask the person for a file: a title and the extensions it may have.
    /// `None` when they cancel, or when there is nobody to ask.
    pub choose_file: &'a dyn Fn(&str, &[String]) -> Option<PathBuf>,
    /// The script's own file, when it was read from one. The script may read
    /// and list anything in that file's folder.
    pub path: Option<PathBuf>,
    /// Show the person a message, as After Effects' `alert` does. With none
    /// the message goes to the log.
    pub alert: Option<&'a dyn Fn(&str)>,
    /// How far the script's saves, footage and exports may go.
    pub limits: Limits,
}

/// How far a script's saves, footage and exports may go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Limits {
    /// Saves and exports into the project's folder or the script's, and
    /// footage from the files the script may read, as File > Run script.
    #[default]
    Script,
    /// Wherever the script says, as on the command line, whose caller is
    /// the person.
    Off,
}

fn nobody_to_ask(_: &str, _: &[String]) -> Option<PathBuf> {
    None
}

impl Default for RunOptions<'_> {
    fn default() -> Self {
        Self {
            name: "Script".into(),
            stop: Arc::default(),
            readable: Vec::new(),
            args: BTreeMap::new(),
            choose_file: &nobody_to_ask,
            path: None,
            alert: None,
            limits: Limits::default(),
        }
    }
}

/// What a run did.
#[derive(Debug, Clone, PartialEq)]
pub struct RunReport {
    /// Every line the script printed, in order.
    pub log: Vec<String>,
    /// The script's last value, or why it did not finish.
    pub result: Result<Value, RunError>,
}

/// Why a run did not finish, and where in the script, when the engine says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunError {
    pub error: ScriptError,
    /// Counted from 1.
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "Line {line}: {}", self.error),
            None => write!(f, "{}", self.error),
        }
    }
}

impl std::error::Error for RunError {}

/// A line and column in the script.
type Place = Option<(usize, usize)>;

/// Run `source` against `host`'s project as one undo step.
pub fn run(host: &dyn Host, source: &str, options: RunOptions<'_>) -> RunReport {
    let log = Arc::new(Mutex::new(Log::default()));
    let desk = Desk {
        host,
        choose_file: options.choose_file,
        alert: options.alert,
        log: log.clone(),
        reach: RefCell::new(Reach {
            folder: options.path.as_deref().and_then(reach::script_folder),
            files: options
                .readable
                .iter()
                .filter_map(|p| p.canonicalize().ok())
                .collect(),
        }),
        limited: options.limits != Limits::Off,
        edited: Cell::new(false),
    };
    let name = if options.name.is_empty() {
        "Script".to_owned()
    } else {
        options.name.clone()
    };
    let setup = Setup {
        name,
        args: options.args,
        path: options
            .path
            .as_deref()
            .and_then(|p| std::path::absolute(p).ok()),
    };
    let stop = options.stop;
    let mut at = None;
    let result = one_step(host.store(), || {
        std::thread::scope(|s| {
            let (ask, asked) = mpsc::sync_channel(0);
            let lumit = Lumit { ask };
            let (setup, stop, log) = (&setup, &stop, &log);
            let worker = std::thread::Builder::new()
                .name("script".into())
                .stack_size(STACK_BYTES)
                .spawn_scoped(s, move || evaluate(source, setup, lumit, stop, log))
                .map_err(|e| ScriptError::Failed(format!("The script could not start: {e}.")))?;
            desk.serve(asked);
            let out = match worker.join() {
                Ok(Ok(value)) => Ok(value),
                Ok(Err((error, place))) => {
                    at = place;
                    Err(error)
                }
                Err(_) => Err(ScriptError::Failed(
                    "The script stopped unexpectedly.".into(),
                )),
            };
            // Stopped means nothing kept, even when the script got to its end
            // before it noticed.
            if stop.load(Ordering::Relaxed) {
                return Err(ScriptError::Stopped);
            }
            // The run goes onto the History list as one step, which reads
            // better named after the script than after its first edit.
            if out.is_ok() && desk.edited.get() {
                host.store().name_undo_group(setup.name.clone());
            }
            out
        })
    });
    let log = log.lock().map(|l| l.lines.clone()).unwrap_or_default();
    RunReport {
        log,
        result: result.map_err(|error| RunError {
            error,
            line: at.map(|(line, _)| line),
            column: at.map(|(_, column)| column),
        }),
    }
}

/// What the script thread is handed besides the source.
struct Setup {
    name: String,
    args: BTreeMap<String, String>,
    /// The script's own file, as `$.fileName` gives it.
    path: Option<PathBuf>,
}

/// A job the script hands back to the caller's thread.
type Job = Box<dyn FnOnce(&Desk<'_>) -> Result<Value, ScriptError> + Send>;

/// The caller's side of a run: the host, and what the script may read.
struct Desk<'a> {
    host: &'a dyn Host,
    choose_file: &'a dyn Fn(&str, &[String]) -> Option<PathBuf>,
    alert: Option<&'a dyn Fn(&str)>,
    log: Arc<Mutex<Log>>,
    /// What the script may read, and where its saves, footage and exports
    /// may go.
    reach: RefCell<Reach>,
    /// Whether its saves, footage and exports keep to the reach.
    limited: bool,
    /// Whether a command changed the document.
    edited: Cell<bool>,
}

impl Desk<'_> {
    /// Run the script's jobs until it finishes, which drops its end.
    fn serve(&self, asked: Receiver<(Job, SyncSender<Result<Value, ScriptError>>)>) {
        for (job, answer) in asked {
            // A script that has gone is not waiting for the answer.
            let _ = answer.send(job(self));
        }
    }

    /// Asked before the file system is, so a path out of reach is refused
    /// the same whether it is there or not.
    fn may_read(&self, path: &Path) -> Result<(), ScriptError> {
        if self.reach.borrow().reads(reach::plain(path)?) {
            return Ok(());
        }
        Err(ScriptError::BadArgs(format!(
            "A script can only read files in its own folder or ones you chose or passed to it, and {} is not one.",
            path.display()
        )))
    }

    fn read(&self, path: &Path) -> Result<String, ScriptError> {
        self.may_read(path)?;
        let full = path.canonicalize().map_err(|_| {
            ScriptError::NotFound(format!("{} could not be found.", path.display()))
        })?;
        lumit_ingress::read_to_string_capped(&full, MAX_READ_BYTES)
            .map_err(|e| ScriptError::Failed(e.to_string()))
    }

    /// Whether `path` is there and whether it is a folder. Anywhere the script
    /// may not look, it is not there, so a script can ask before it reads.
    fn look(&self, path: &Path) -> Value {
        let full = match self.may_read(path) {
            Ok(()) => path.canonicalize().ok(),
            Err(_) => None,
        };
        match full {
            Some(full) => json!({ "exists": true, "folder": full.is_dir() }),
            None => json!({ "exists": false, "folder": false }),
        }
    }

    /// The names in folder `path`, folders marked, in name order.
    fn list(&self, path: &Path) -> Result<Value, ScriptError> {
        self.may_read(path)?;
        let full = path.canonicalize().map_err(|_| {
            ScriptError::NotFound(format!("{} could not be found.", path.display()))
        })?;
        let entries = std::fs::read_dir(&full).map_err(|e| {
            ScriptError::Failed(format!("{} could not be read: {e}.", path.display()))
        })?;
        let mut found: Vec<(String, bool)> = entries
            .filter_map(Result::ok)
            .take(MAX_LISTED)
            .map(|e| {
                let folder = e.file_type().is_ok_and(|t| t.is_dir());
                (e.file_name().to_string_lossy().into_owned(), folder)
            })
            .collect();
        found.sort();
        Ok(found
            .into_iter()
            .map(|(name, folder)| json!({ "name": name, "folder": folder }))
            .collect())
    }
}

/// The script thread's way back to the caller's.
#[derive(Clone)]
struct Lumit {
    ask: SyncSender<(Job, SyncSender<Result<Value, ScriptError>>)>,
}

impl Lumit {
    /// Run `job` on the caller's thread and wait for its answer.
    fn ask(
        &self,
        job: impl FnOnce(&Desk<'_>) -> Result<Value, ScriptError> + Send + 'static,
    ) -> Result<Value, ScriptError> {
        let (answer, answered) = mpsc::sync_channel(1);
        let gone = || ScriptError::Failed("Lumit is no longer listening.".into());
        self.ask.send((Box::new(job), answer)).map_err(|_| gone())?;
        answered.recv().map_err(|_| gone())?
    }

    fn call(&self, name: String, args: Value) -> Result<Value, ScriptError> {
        self.ask(move |desk| {
            // The run is already one step, so undo here would take back
            // something from before it.
            if matches!(name.as_str(), "undo" | "redo") {
                return Err(ScriptError::BadArgs(
                    "A script cannot undo or redo. Its whole run is one step.".into(),
                ));
            }
            let store = desk.host.store();
            let before = store.revision();
            let reach = desk.reach.borrow();
            let reach = desk.limited.then_some(&*reach);
            let out = commands::run(desk.host, reach, &name, args);
            if store.revision() != before {
                desk.edited.set(true);
            }
            out
        })
    }
}

/// What a script printed, and how many bytes of it.
#[derive(Default)]
struct Log {
    lines: Vec<String>,
    bytes: usize,
}

fn keep(log: &Mutex<Log>, line: String) {
    let line = cut(line);
    if let Ok(mut log) = log.lock() {
        if log.lines.len() < MAX_LOG_LINES && log.bytes + line.len() <= MAX_LOG_BYTES {
            log.bytes += line.len();
            log.lines.push(line);
        }
    }
}

/// `text` cut to [`MAX_LINE_BYTES`], with an ellipsis where it was cut.
fn cut(text: String) -> String {
    let mut end = MAX_LINE_BYTES;
    if text.len() <= end {
        return text;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", text.get(..end).unwrap_or_default())
}

/// Run the script on this thread, and answer its last value.
fn evaluate(
    source: &str,
    setup: &Setup,
    lumit: Lumit,
    stop: &Arc<AtomicBool>,
    log: &Arc<Mutex<Log>>,
) -> Result<Value, (ScriptError, Place)> {
    let broken = |e: rquickjs::Error| {
        (
            ScriptError::Failed(format!("The script could not start: {e}.")),
            None,
        )
    };
    let runtime = Runtime::new().map_err(broken)?;
    runtime.set_memory_limit(MAX_MEMORY);
    runtime.set_max_stack_size(ENGINE_STACK_BYTES);
    let stop = stop.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || stop.load(Ordering::Relaxed))));
    let context = Context::full(&runtime).map_err(broken)?;
    let source = without_directives(source)?;
    context.with(|ctx| {
        // The command error a native call last threw, so a script that lets
        // it fly comes back with the error it was rather than its text.
        let thrown = Rc::new(RefCell::new(None));
        let natives = natives(&ctx, &lumit, log, &thrown);
        let outcome = natives.and_then(|n| {
            let prelude: Function = ctx.eval_with_options(LUMIT_JS, options("<lumit>"))?;
            let commands = json!(COMMANDS.iter().map(|c| c.name).collect::<Vec<_>>());
            prelude.call::<_, ()>((
                n.clone(),
                ctx.json_parse(commands.to_string())?,
                ctx.json_parse(json!(setup.args).to_string())?,
            ))?;
            let prelude: Function = ctx.eval_with_options(AE_JS, options("<after-effects>"))?;
            let path = setup
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned());
            prelude.call::<_, ()>((n.clone(), path, cfg!(windows)))?;
            let value: JsValue = ctx.eval_with_options(source.as_str(), options(&setup.name))?;
            if let Ok(finish) = n.get::<_, Function>("finish") {
                finish.call::<_, ()>(())?;
            }
            let text = ctx
                .json_stringify(value)
                .ok()
                .flatten()
                .and_then(|s| s.to_string().ok());
            Ok(text
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null))
        });
        let error = thrown.borrow_mut().take();
        outcome
            .catch(&ctx)
            .map_err(|e| failure(&ctx, e, error, &setup.name))
    })
}

fn options(file: &str) -> EvalOptions {
    let mut o = EvalOptions::default();
    // After Effects scripts are written for the old, loose rules.
    o.strict = false;
    o.filename = Some(file.to_owned());
    o
}

/// The functions `lumit.js` builds the `lumit` object over.
fn natives<'js>(
    ctx: &Ctx<'js>,
    lumit: &Lumit,
    log: &Arc<Mutex<Log>>,
    thrown: &Rc<RefCell<Option<ScriptError>>>,
) -> rquickjs::Result<Object<'js>> {
    let n = Object::new(ctx.clone())?;

    // Every job answers JSON text, and an error is thrown into the script.
    let asking = |job: fn(&Lumit, String, String) -> Result<Value, ScriptError>| {
        let (lumit, thrown) = (lumit.clone(), thrown.clone());
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, a: String, b: Opt<String>| -> rquickjs::Result<String> {
                job(&lumit, a, b.0.unwrap_or_default())
                    .map(|v| v.to_string())
                    .map_err(|e| throw(&ctx, &thrown, e))
            },
        )
    };
    n.set(
        "call",
        asking(|lumit, name, args| {
            let args =
                serde_json::from_str(&args).map_err(|e| ScriptError::BadArgs(e.to_string()))?;
            lumit.call(name, args)
        })?,
    )?;
    n.set(
        "chooseFile",
        asking(|lumit, title, extensions| {
            let extensions: Vec<String> = serde_json::from_str(&extensions).unwrap_or_default();
            lumit.ask(move |desk| {
                let Some(path) = (desk.choose_file)(&title, &extensions) else {
                    return Ok(Value::Null);
                };
                if let Ok(full) = path.canonicalize() {
                    desk.reach.borrow_mut().files.push(full);
                }
                Ok(json!(path.to_string_lossy()))
            })
        })?,
    )?;
    n.set(
        "readText",
        asking(|lumit, path, _| {
            lumit.ask(move |desk| desk.read(Path::new(&path)).map(Value::String))
        })?,
    )?;
    n.set(
        "exists",
        asking(|lumit, path, _| lumit.ask(move |desk| Ok(desk.look(Path::new(&path)))))?,
    )?;
    n.set(
        "list",
        asking(|lumit, path, _| lumit.ask(move |desk| desk.list(Path::new(&path))))?,
    )?;
    n.set(
        "alert",
        asking(|lumit, text, _| {
            lumit.ask(move |desk| {
                match desk.alert {
                    Some(alert) => alert(&cut(text)),
                    None => keep(&desk.log, text),
                }
                Ok(Value::Null)
            })
        })?,
    )?;

    let printed = log.clone();
    n.set(
        "log",
        Function::new(ctx.clone(), move |line: String| keep(&printed, line))?,
    )?;
    let thrown = thrown.clone();
    let rate_thrown = thrown.clone();
    n.set(
        "rateOf",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, fps: f64| -> rquickjs::Result<String> {
                FrameRate::from_decimal(fps)
                    .map(|r| json!({ "num": r.num(), "den": r.den() }).to_string())
                    .ok_or_else(|| {
                        let e = ScriptError::BadArgs(format!("{fps} is not a frame rate."));
                        throw(&ctx, &rate_thrown, e)
                    })
            },
        )?,
    )?;
    n.set(
        "frameOn",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, seconds: f64, num: u32, den: u32| -> rquickjs::Result<f64> {
                frame_on(seconds, num, den)
                    .map(|frame| frame as f64)
                    .map_err(|e| throw(&ctx, &thrown, e))
            },
        )?,
    )?;
    Ok(n)
}

/// The frame nearest another program's time in `seconds`, on a rate of
/// `num / den` frames a second. Nearest rather than the one it falls in,
/// since a sample meant for frame 1 can arrive a hair early (0.0166666 at 60
/// fps).
fn frame_on(seconds: f64, num: u32, den: u32) -> Result<i64, ScriptError> {
    let rate = FrameRate::new(num, den)
        .map_err(|_| ScriptError::BadArgs("That frame rate is nought.".into()))?;
    let t = Rational::from_f64_on_grid(seconds, Rational::FLICK_DEN)
        .map_err(|_| ScriptError::BadArgs("That time is out of range.".into()))?;
    Ok(rate.nearest_frame_at(CompTime(t)))
}

/// `source` with After Effects' `#target` lines and their like blanked, as
/// they are not JavaScript, so the line numbers stay the script's own.
/// `#include` pulls in another file, which a run does not.
fn without_directives(source: &str) -> Result<String, (ScriptError, Place)> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut out = String::with_capacity(source.len());
    for (n, line) in source.split_inclusive('\n').enumerate() {
        let word = line
            .trim_start()
            .strip_prefix('#')
            .and_then(|rest| rest.split_whitespace().next());
        match word {
            Some("target" | "targetengine" | "strict" | "script" | "engine") => {
                out.push_str(&line[line.trim_end_matches(['\r', '\n']).len()..]);
            }
            Some("include" | "includepath") => {
                return Err((
                    ScriptError::BadArgs(
                        "#include is not supported in Lumit, which runs a script as one file."
                            .into(),
                    ),
                    Some((n + 1, 1)),
                ));
            }
            _ => out.push_str(line),
        }
    }
    Ok(out)
}

/// Throw `error` into the script as an `Error` carrying its `code`.
fn throw(
    ctx: &Ctx<'_>,
    thrown: &RefCell<Option<ScriptError>>,
    error: ScriptError,
) -> rquickjs::Error {
    let message = error.to_string();
    let code = error.code();
    *thrown.borrow_mut() = Some(error);
    match Exception::from_message(ctx.clone(), &message) {
        Ok(e) => {
            let _ = e.as_object().set("code", code);
            e.throw()
        }
        Err(e) => e,
    }
}

/// Why the script failed, and where in it. `thrown` is the last command
/// error, which is what the script failed with if the text matches.
fn failure<'js>(
    ctx: &Ctx<'js>,
    error: CaughtError<'js>,
    thrown: Option<ScriptError>,
    file: &str,
) -> (ScriptError, Place) {
    match error {
        CaughtError::Exception(e) => {
            let message = e.message().unwrap_or_default();
            let kind: Option<String> = e.get("name").ok();
            let at = e.stack().and_then(|s| place(&s, file));
            let error = match thrown {
                Some(t) if t.to_string() == message => t,
                _ => ScriptError::Failed(match kind.as_deref() {
                    None | Some("" | "Error") => message,
                    Some(kind) => format!("{kind}: {message}"),
                }),
            };
            (error, at)
        }
        CaughtError::Value(v) => {
            let text = Coerced::<String>::from_js(ctx, v).map_or_else(|e| e.to_string(), |s| s.0);
            (ScriptError::Failed(text), None)
        }
        CaughtError::Error(e) => (ScriptError::Failed(e.to_string()), None),
    }
}

/// The first place in `stack` that is in the script itself rather than in
/// Lumit's own JavaScript. A frame reads `at name (file:line:column)`.
fn place(stack: &str, file: &str) -> Place {
    stack.lines().find_map(|frame| {
        let frame = frame.trim().strip_prefix("at ")?;
        let inner = frame
            .rsplit_once('(')
            .map_or(frame, |(_, inner)| inner)
            .trim_end_matches(')');
        let mut parts = inner.rsplitn(3, ':');
        let column = parts.next()?.parse().ok()?;
        let line = parts.next()?.parse().ok()?;
        (parts.next()? == file).then_some((line, column))
    })
}
