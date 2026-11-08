//! The 3D model files Lumit has no reader for, opened through the user's own
//! Blender, which turns each one into a GLB the glTF reader opens.
//!
//! Blender never ships with Lumit. It is found on the machine, or picked in
//! Settings, and runs in the background as a child process. The GLB is kept in
//! the cache under the source's length, modified time and the Blender version,
//! so a file saved over converts again and an unchanged one opens at once.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::SceneError;

/// What goes through Blender, by lower-case extension. FBX does too, when the
/// native reader refuses one.
pub const EXTENSIONS: [&str; 9] = [
    "blend", "usd", "usdz", "usdc", "usda", "abc", "ply", "stl", "dae",
];

/// The script Blender runs. Part of the cache key, so a change to it converts
/// everything again.
const SCRIPT: &str = include_str!("to_glb.py");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blender {
    pub exe: PathBuf,
    /// As it names itself, "5.2.3".
    pub version: String,
}

/// Where a source's conversion stands, while it isn't simply done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Converting,
    NoBlender,
    /// What Blender printed about it.
    Failed(String),
    /// This Blender has no importer for the file's type.
    NoImporter,
    /// Stopped by hand, and left until it's asked for again.
    Stopped,
}

/// The program picked in Settings, tried before anything on the machine.
static CHOSEN: Mutex<Option<PathBuf>> = Mutex::new(None);
/// The last Blender found, so each conversion doesn't go looking again.
static FOUND: Mutex<Option<Blender>> = Mutex::new(None);
/// Each source being converted, or whose last conversion failed.
static STATES: Mutex<Option<HashMap<PathBuf, State>>> = Mutex::new(None);

/// Moves each time [`choose`] or [`convert_again`] is called.
static CHOICES: AtomicU64 = AtomicU64::new(0);

/// The most Blenders run at once: each is a whole program, so ten files
/// dropped together wait their turn.
const AT_ONCE: usize = 2;
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Use `exe` before any Blender found on the machine, or stop when `None`.
pub fn choose(exe: Option<PathBuf>) {
    if let Ok(mut chosen) = CHOSEN.lock() {
        *chosen = exe;
    }
    if let Ok(mut found) = FOUND.lock() {
        *found = None;
    }
    CHOICES.fetch_add(1, Ordering::Relaxed);
}

/// A number that moves when a Blender is chosen, so a file that failed for
/// want of one is tried again.
pub fn choices() -> u64 {
    CHOICES.load(Ordering::Relaxed)
}

/// The Blender conversions run: the one chosen if it answers, else the first
/// found that does. Each is asked for `--version`, since an old file
/// association can point at a program that is long gone.
pub fn find() -> Option<Blender> {
    let held = FOUND.lock().ok().and_then(|found| found.clone());
    if let Some(held) = held.filter(|held| held.exe.is_file()) {
        return Some(held);
    }
    let chosen = CHOSEN.lock().ok().and_then(|chosen| chosen.clone());
    let found = chosen
        .into_iter()
        .chain(std::iter::once_with(candidates).flatten())
        .map(program_in)
        .find_map(|exe| {
            let version = version(&exe)?;
            Some(Blender { exe, version })
        })?;
    if let Ok(mut held) = FOUND.lock() {
        *held = Some(found.clone());
    }
    Some(found)
}

/// Where `source`'s conversion stands, `None` before one starts and once it
/// is done.
pub fn state(source: &Path) -> Option<State> {
    STATES.lock().ok()?.as_ref()?.get(source).cloned()
}

fn set_state(source: &Path, state: Option<State>) {
    if let Ok(mut states) = STATES.lock() {
        let states = states.get_or_insert_with(HashMap::new);
        match state {
            Some(state) => states.insert(source.to_path_buf(), state),
            None => states.remove(source),
        };
    }
}

/// Stop `source`'s conversion. Blender is killed by its own handle, and the
/// file isn't tried again until it's saved or [`convert_again`] is called.
pub fn stop(source: &Path) {
    stop_where(&|path| path == source);
}

/// Stop every conversion of a file `which` picks.
pub fn stop_where(which: &dyn Fn(&Path) -> bool) {
    if let Ok(mut states) = STATES.lock() {
        for (path, state) in states.get_or_insert_with(HashMap::new) {
            if *state == State::Converting && which(path) {
                *state = State::Stopped;
            }
        }
    }
}

/// Wait a moment for the Blenders stopped to be gone, as the app quits.
pub fn wait_for_stopped() {
    let deadline = Instant::now() + Duration::from_secs(2);
    while RUNNING.load(Ordering::Relaxed) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Try `source` again after it was stopped. Moves [`choices`], as that's what
/// names a read and its frames, so a frame drawn without it isn't served.
pub fn convert_again(source: &Path) {
    set_state(source, None);
    CHOICES.fetch_add(1, Ordering::Relaxed);
}

/// Wait for room among the Blenders running and take it. False when `stopped`
/// comes first.
fn take_room(stopped: &dyn Fn() -> bool) -> bool {
    loop {
        if stopped() {
            return false;
        }
        let room = |n: usize| (n < AT_ONCE).then_some(n + 1);
        if RUNNING
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, room)
            .is_ok()
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A GLB from an earlier conversion of `place`'s file, as long as the file
/// hasn't been saved since.
fn converted_before(cache_dir: &Path, place: u64, source: &std::fs::Metadata) -> Option<PathBuf> {
    let prefix = format!("{place:016x}-");
    let saved = source.modified().ok()?;
    std::fs::read_dir(cache_dir)
        .ok()?
        .flatten()
        .find(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix)
                && name.ends_with(".glb")
                && is_glb(&entry.path())
                && entry
                    .metadata()
                    .and_then(|glb| glb.modified())
                    .is_ok_and(|made| made >= saved)
        })
        .map(|entry| entry.path())
}

/// Turn `source` into a GLB in `cache_dir` and give its path, once Blender has
/// finished, whose first launch can take a while. A source already being
/// converted is waited for rather than converted twice. Setting `cancel`
/// stops Blender.
pub fn to_glb(source: &Path, cache_dir: &Path, cancel: &AtomicBool) -> Result<PathBuf, SceneError> {
    let ext = source
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !EXTENSIONS.contains(&ext.as_str()) && ext != "fbx" {
        return Err(SceneError::Unsupported {
            path: source.to_path_buf(),
        });
    }
    let meta = std::fs::metadata(source).map_err(|e| SceneError::Read {
        path: source.to_path_buf(),
        reason: e.to_string(),
    })?;
    let mut hash = DefaultHasher::new();
    source.hash(&mut hash);
    let place = hash.finish();
    let Some(blender) = find() else {
        if let Some(glb) = converted_before(cache_dir, place, &meta) {
            set_state(source, None);
            return Ok(glb);
        }
        set_state(source, Some(State::NoBlender));
        return Err(SceneError::NoBlender {
            path: source.to_path_buf(),
        });
    };

    (meta.len(), meta.modified().ok(), &blender.version, SCRIPT).hash(&mut hash);
    let glb = cache_dir.join(format!("{place:016x}-{:016x}.glb", hash.finish()));
    let mut waited = false;
    loop {
        if is_glb(&glb) {
            return Ok(glb);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(SceneError::Cancelled);
        }
        // Another thread converting this source is waited for, and what stops
        // it stops this one too.
        let state = STATES.lock().map(|mut states| {
            let states = states.get_or_insert_with(HashMap::new);
            let state = states.get(source).cloned();
            if state != Some(State::Converting) && !(waited && state.is_some()) {
                states.insert(source.to_path_buf(), State::Converting);
                return None;
            }
            state
        });
        match state {
            Ok(Some(State::Converting)) => waited = true,
            Ok(Some(_)) => {
                return Err(SceneError::Read {
                    path: source.to_path_buf(),
                    reason: "its conversion didn't finish".into(),
                })
            }
            _ => break,
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let stopped = || cancel.load(Ordering::Relaxed) || state(source) == Some(State::Stopped);
    let done = if take_room(&stopped) {
        let done =
            convert(source, &ext, &blender.exe, cache_dir, place, &stopped).and_then(|part| {
                std::fs::rename(&part, &glb).map_err(|e| SceneError::Read {
                    path: source.to_path_buf(),
                    reason: e.to_string(),
                })?;
                forget_older(cache_dir, place, &glb);
                Ok(glb)
            });
        RUNNING.fetch_sub(1, Ordering::Relaxed);
        done
    } else {
        Err(SceneError::Cancelled)
    };
    // Stopped by hand counts as a read that failed, so it's kept as one and
    // nothing starts Blender again on the next frame.
    if state(source) == Some(State::Stopped) && done.is_err() {
        return Err(SceneError::Read {
            path: source.to_path_buf(),
            reason: "stopped".into(),
        });
    }
    set_state(
        source,
        match &done {
            Err(SceneError::Read { reason, .. }) => Some(State::Failed(reason.clone())),
            Err(SceneError::Unsupported { .. }) => Some(State::NoImporter),
            _ => None,
        },
    );
    done
}

/// One run of Blender, giving the GLB it wrote under a name of its own, so a
/// half-written file is never taken for a finished one.
fn convert(
    source: &Path,
    ext: &str,
    exe: &Path,
    cache_dir: &Path,
    place: u64,
    stopped: &dyn Fn() -> bool,
) -> Result<PathBuf, SceneError> {
    let failed = |reason: String| SceneError::Read {
        path: source.to_path_buf(),
        reason,
    };
    let run_name = format!("{place:016x}.part-{}", unique());
    let part = cache_dir.join(format!("{run_name}.glb"));
    let log = cache_dir.join(format!("{run_name}.log"));
    std::fs::create_dir_all(cache_dir).map_err(|e| failed(e.to_string()))?;

    let mut command = Command::new(exe);
    command.args(["-b", "--factory-startup"]);
    if ext == "blend" {
        command.arg(source);
    }
    command
        .args(["--python-exit-code", "1", "--python-expr", SCRIPT, "--"])
        .arg(source)
        .arg(&part);
    let status = run(&mut command, &log, stopped, None);
    let text = String::from_utf8_lossy(&std::fs::read(&log).unwrap_or_default()).into_owned();
    let _ = std::fs::remove_file(&log);
    match status.map_err(|e| failed(e.to_string()))? {
        Some(done) if done.success() && is_glb(&part) => Ok(part),
        stopped => {
            let _ = std::fs::remove_file(&part);
            Err(match stopped {
                None => SceneError::Cancelled,
                Some(done) if done.code() == Some(3) => SceneError::Unsupported {
                    path: source.to_path_buf(),
                },
                Some(_) => failed(tail(&text)),
            })
        }
    }
}

/// Open `source` in Blender's own window, as the user would. False when no
/// Blender was found or it wouldn't start.
pub fn edit(source: &Path) -> bool {
    let Some(blender) = find() else {
        return false;
    };
    let mut command = Command::new(&blender.exe);
    no_console(&mut command);
    command
        .arg(source)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// A name no other run in any Lumit uses at the same time.
fn unique() -> String {
    static RUNS: AtomicU64 = AtomicU64::new(0);
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    format!("{}-{run}", std::process::id())
}

/// GLBs of earlier saves of the same source, which nothing will ask for again.
fn forget_older(cache_dir: &Path, place: u64, keep: &Path) {
    let prefix = format!("{place:016x}-");
    for entry in std::fs::read_dir(cache_dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && name.ends_with(".glb") && path != keep {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// What Blender printed that says why it failed: its error lines less their
/// timestamps, or with none its last lines.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("Blender "))
        .collect();
    let mut errors: Vec<&str> = lines
        .iter()
        .filter(|line| line.contains("Error:") || line.contains("ERROR"))
        .map(|line| line.rsplit_once("| ").map_or(*line, |(_, rest)| rest))
        .collect();
    errors.dedup();
    let lines = if errors.is_empty() { lines } else { errors };
    lines[lines.len().saturating_sub(6)..].join("\n")
}

fn is_glb(path: &Path) -> bool {
    let mut magic = [0; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && &magic == b"glTF"
}

/// Blender's version, or `None` when `exe` isn't a Blender that runs.
fn version(exe: &Path) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    let log = std::env::temp_dir().join(format!("lumit-blender-{}.log", unique()));
    let deadline = Instant::now() + Duration::from_secs(20);
    let ran = run(
        Command::new(exe).arg("--version"),
        &log,
        &|| false,
        Some(deadline),
    );
    let text = String::from_utf8_lossy(&std::fs::read(&log).unwrap_or_default()).into_owned();
    let _ = std::fs::remove_file(&log);
    if !matches!(ran, Ok(Some(status)) if status.success()) {
        return None;
    }
    let version = text
        .lines()
        .find_map(|line| line.strip_prefix("Blender "))?;
    let version = version.split_whitespace().next()?;
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| version.to_string())
}

/// Run `command` to the end with everything it prints going to `log`. `None`
/// when `stopped` or `deadline` stopped it first, which kills it by its own
/// handle.
fn run(
    command: &mut Command,
    log: &Path,
    stopped: &dyn Fn() -> bool,
    deadline: Option<Instant>,
) -> std::io::Result<Option<ExitStatus>> {
    let out = std::fs::File::create(log)?;
    no_console(command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out)
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if stopped() || deadline.is_some_and(|at| Instant::now() >= at) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Start a console program with no window of its own, which would otherwise
/// pop up in front of the editor on Windows.
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

/// The program itself when `path` is its folder, a Mac app or the Steam
/// launcher beside it.
fn program_in(path: PathBuf) -> PathBuf {
    let exe = if cfg!(windows) {
        "blender.exe"
    } else {
        "blender"
    };
    if path.extension().is_some_and(|ext| ext == "app") {
        path.join("Contents/MacOS/Blender")
    } else if path.is_dir() {
        path.join(exe)
    } else if path
        .file_name()
        .is_some_and(|name| name == "blender-launcher.exe")
    {
        path.with_file_name(exe)
    } else {
        path
    }
}

/// Where Blender may be on this machine, most likely first.
fn candidates() -> Vec<PathBuf> {
    let exe = if cfg!(windows) {
        "blender.exe"
    } else {
        "blender"
    };
    let on_path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(exe))
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        use windows_registry::{CLASSES_ROOT, CURRENT_USER, LOCAL_MACHINE};
        // Blender's installer leaves its folder in the uninstall list.
        let uninstall = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
        let mut installed = Vec::new();
        for list in [LOCAL_MACHINE, CURRENT_USER]
            .into_iter()
            .filter_map(|root| root.open(uninstall).ok())
        {
            installed.extend(list.keys().into_iter().flatten().filter_map(|name| {
                let entry = list.open(name).ok()?;
                let ours = entry.get_string("Publisher").ok()? == "Blender Foundation";
                ours.then(|| entry.get_string("InstallLocation").ok().map(PathBuf::from))?
            }));
        }
        out.extend(newest_first(installed));
        if let Some(programs) = std::env::var_os("ProgramFiles") {
            let dir = Path::new(&programs).join("Blender Foundation");
            let folders = std::fs::read_dir(dir).into_iter().flatten().flatten();
            out.extend(newest_first(folders.map(|entry| entry.path()).collect()));
        }
        let steam = CURRENT_USER
            .open(r"Software\Valve\Steam")
            .and_then(|key| key.get_string("SteamPath"))
            .map_or_else(
                |_| PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from,
            );
        out.extend(
            steam_libraries(&steam)
                .into_iter()
                .map(|lib| lib.join(r"steamapps\common\Blender")),
        );
        // The program .blend files open with, the first word of its command.
        if let Ok(open) = CLASSES_ROOT
            .open(r"blendfile\shell\open\command")
            .and_then(|key| key.get_string(""))
        {
            let open = open.trim_start_matches('"');
            out.push(open.split('"').next().unwrap_or(open).into());
        }
        out.extend(on_path);
    }
    #[cfg(target_os = "macos")]
    {
        out.push("/Applications/Blender.app".into());
        out.extend(
            std::env::var_os("HOME").map(|home| Path::new(&home).join("Applications/Blender.app")),
        );
        out.extend(on_path);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        out.extend(on_path);
        out.push("/snap/bin/blender".into());
        out.push("/var/lib/flatpak/exports/bin/org.blender.Blender".into());
        out.extend(std::env::var_os("HOME").map(|home| {
            Path::new(&home).join(".local/share/flatpak/exports/bin/org.blender.Blender")
        }));
    }
    out
}

/// The folders named `Blender 5.2`, `Blender 4.5` and so on in `folders`,
/// highest version first.
#[cfg(windows)]
fn newest_first(folders: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut found: Vec<(Vec<u32>, PathBuf)> = folders
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            let version = name.strip_prefix("Blender ")?;
            let version = version.split('.').map(|n| n.parse().unwrap_or(0)).collect();
            Some((version, path))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, path)| path).collect()
}

/// Steam's own folder and every library its `libraryfolders.vdf` names.
#[cfg(windows)]
fn steam_libraries(steam: &Path) -> Vec<PathBuf> {
    let vdf =
        std::fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")).unwrap_or_default();
    let mut out = vec![steam.to_path_buf()];
    out.extend(vdf.lines().filter_map(|line| {
        let path = line
            .trim()
            .strip_prefix("\"path\"")?
            .trim()
            .trim_matches('"');
        Some(PathBuf::from(path.replace(r"\\", r"\")))
    }));
    out
}
