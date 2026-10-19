//! Where a script may reach on disk.
//!
//! Every path a caller hands over is read as text first: a network share, a
//! device or anything else that is not a plain path on this computer is
//! refused before the file system is asked anything, so a path cannot make
//! Lumit knock on another machine. Then the path has to be in reach, which is
//! asked of the nearest part of it that is there, so a path outside answers
//! the same refusal whether it exists or not.
//!
//! In reach: the script's own folder and the files the person chose or passed
//! to it for reading, and the project's folder and the script's for saves and
//! exports. A save goes over the project's own file or makes a new `.lum`, and
//! an export never writes over a file.

use std::path::{Component, Path, PathBuf};

use crate::{Host, ScriptError};

/// What a run may reach besides the project itself.
#[derive(Debug, Default)]
pub(crate) struct Reach {
    /// The script's own folder. Anything under it may be read, and saves and
    /// exports may go into it.
    pub(crate) folder: Option<PathBuf>,
    /// Files the person chose or passed in, as real paths.
    pub(crate) files: Vec<PathBuf>,
}

/// `path` when it is a plain path on this computer. A network share, a
/// device, a stream of a file or a raw system path is refused, by its text
/// alone.
pub(crate) fn plain(path: &Path) -> Result<&Path, ScriptError> {
    let refused = || {
        ScriptError::BadArgs(format!(
            "{} is not a file on this computer.",
            path.display()
        ))
    };
    let text = path.to_string_lossy();
    if text.is_empty() || text.contains('\0') {
        return Err(refused());
    }
    #[cfg(windows)]
    {
        let separator = |c: char| c == '\\' || c == '/';
        let mut chars = text.chars();
        // \\server, \\?\ and \\.\ all start with two separators, and \??\
        // is the system's own way in.
        let first = chars.next().is_some_and(separator);
        if first && (chars.next().is_some_and(separator) || text.get(1..3) == Some("??")) {
            return Err(refused());
        }
        for part in path.components() {
            let fine = match part {
                Component::Prefix(p) => {
                    matches!(p.kind(), std::path::Prefix::Disk(_)) && path.has_root()
                }
                Component::Normal(name) => {
                    let name = name.to_string_lossy();
                    !name.contains(':') && !device(&name)
                }
                _ => true,
            };
            if !fine {
                return Err(refused());
            }
        }
    }
    Ok(path)
}

/// Whether `name` is one Windows opens as a device wherever it sits, such as
/// NUL or COM1.
#[cfg(windows)]
fn device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let stem = stem.to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.chars().count() == 4
        && stem
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c));
    numbered
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
}

/// The folder a script at `script` may read and write in: its own, unless
/// that is the root of a drive or the home folder itself, which would open
/// up everything. Its subfolders come with it.
pub(crate) fn script_folder(script: &Path) -> Option<PathBuf> {
    let folder = std::path::absolute(script).ok()?.parent()?.to_path_buf();
    let real = folder.canonicalize().ok()?;
    let home = std::env::home_dir().and_then(|h| h.canonicalize().ok());
    (real.parent().is_some() && Some(&real) != home.as_ref()).then_some(folder)
}

/// `path`, absolute and without `.` or `..`, or `None` when it climbs above
/// its root.
fn whole(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            part => out.push(part),
        }
    }
    Some(out)
}

/// Whether whole path `path` is in one of `folders`. Asked of the nearest
/// part of it that is there, so a path that is not there gets the same
/// answer as its folder.
fn inside(path: &Path, folders: &[PathBuf]) -> bool {
    let Some(real) = path.ancestors().find_map(|p| p.canonicalize().ok()) else {
        return false;
    };
    folders
        .iter()
        .filter_map(|f| f.canonicalize().ok())
        .any(|f| real.starts_with(f))
}

fn unsaved() -> ScriptError {
    ScriptError::BadArgs("The project has not been saved yet. Save it in Lumit first.".into())
}

impl Reach {
    /// Whether a script may read or look at `path`: in its folder, or a file
    /// chosen or passed. A relative path is read from where Lumit runs.
    pub(crate) fn reads(&self, path: &Path) -> bool {
        let Some(path) = std::path::absolute(path).ok().and_then(|p| whole(&p)) else {
            return false;
        };
        let chosen = path.canonicalize().is_ok_and(|p| self.files.contains(&p));
        chosen || inside(&path, self.folder.as_slice())
    }

    /// `path` made whole, a relative one read from the script's folder or
    /// else the project's.
    fn whole(&self, host: &dyn Host, path: &Path) -> Option<PathBuf> {
        let project = host.path().and_then(|p| p.parent().map(Path::to_path_buf));
        let base = self.folder.clone().or(project);
        let path = match base {
            _ if path.is_absolute() => path.to_path_buf(),
            Some(base) => base.join(path),
            None => return None,
        };
        whole(&path)
    }

    /// The folders saves and exports may go into.
    fn writes(&self, host: &dyn Host) -> Result<Vec<PathBuf>, ScriptError> {
        let project = host.path().and_then(|p| p.parent().map(Path::to_path_buf));
        let folders: Vec<PathBuf> = project.into_iter().chain(self.folder.clone()).collect();
        if folders.is_empty() {
            return Err(unsaved());
        }
        Ok(folders)
    }

    fn write_places(&self) -> &'static str {
        if self.folder.is_some() {
            "the project's folder or the script's"
        } else {
            "the project's folder"
        }
    }

    /// Footage file `path` as the host is to read it, or why it may not.
    pub(crate) fn footage(&self, host: &dyn Host, path: &Path) -> Result<PathBuf, ScriptError> {
        if let Some(full) = self.whole(host, path) {
            let chosen = full.canonicalize().is_ok_and(|p| self.files.contains(&p));
            if chosen || inside(&full, self.folder.as_slice()) {
                return Ok(full);
            }
        }
        let places = if self.folder.is_some() {
            "the script's folder or files you chose or passed to it"
        } else {
            "files you chose or passed to the script"
        };
        Err(ScriptError::BadArgs(format!(
            "{} is out of reach. Footage comes only from {places}.",
            path.display()
        )))
    }

    /// Where a save to `path`, or to the project's own file when that is
    /// `None`, may go: over that file, or to a new `.lum` file in reach.
    pub(crate) fn save(
        &self,
        host: &dyn Host,
        path: Option<&Path>,
    ) -> Result<Option<PathBuf>, ScriptError> {
        let folders = self.writes(host)?;
        let Some(path) = path else {
            return Ok(None);
        };
        let out_of_reach = || {
            ScriptError::BadArgs(format!(
                "{} is out of reach. The project saves over its own file, or as a new .lum file in {}.",
                path.display(),
                self.write_places()
            ))
        };
        let mut full = self.whole(host, path).ok_or_else(out_of_reach)?;
        // A name with no extension is a name, as in Lumit's own save.
        if full.extension().is_none() {
            full.set_extension("lum");
        }
        let own = host.path().and_then(|p| p.canonicalize().ok());
        if own.is_some() && full.canonicalize().ok() == own {
            return Ok(Some(full));
        }
        if !full
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lum"))
        {
            return Err(ScriptError::BadArgs(
                "A project saves as a .lum file.".into(),
            ));
        }
        if !inside(&full, &folders) {
            return Err(out_of_reach());
        }
        // Asked of a link itself, so one to a file elsewhere is not written
        // through either.
        if full.symlink_metadata().is_ok() {
            return Err(ScriptError::BadArgs(format!(
                "{} is already there. Only the project's own file is saved over.",
                path.display()
            )));
        }
        Ok(Some(full))
    }

    /// Where an export to `out` may go: into a folder in reach, never over a
    /// file. `None` leaves it to the host, which puts it beside the project.
    pub(crate) fn export(
        &self,
        host: &dyn Host,
        out: Option<&Path>,
    ) -> Result<Option<PathBuf>, ScriptError> {
        let folders = self.writes(host)?;
        let Some(out) = out else {
            return Ok(None);
        };
        let out_of_reach = || {
            ScriptError::BadArgs(format!(
                "{} is out of reach. Exports go only into {}.",
                out.display(),
                self.write_places()
            ))
        };
        let mut full = self.whole(host, out).ok_or_else(out_of_reach)?;
        if full.extension().is_none() {
            full.set_extension("mp4");
        }
        if !inside(&full, &folders) {
            return Err(out_of_reach());
        }
        if full.symlink_metadata().is_ok() {
            return Err(ScriptError::BadArgs(format!(
                "{} is already there, and an export never writes over a file.",
                full.display()
            )));
        }
        Ok(Some(full))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // The promise behind these: a script reaches its own folder and nothing
    // past it, and never another machine.
    #[test]
    fn paths_keep_to_the_reach() {
        let dir = std::env::temp_dir().join(format!("reach-{}", uuid::Uuid::now_v7()));
        let folder = dir.join("script");
        std::fs::create_dir_all(folder.join("worldpass")).unwrap();
        let reach = Reach {
            folder: script_folder(&folder.join("go.jsx")),
            ..Reach::default()
        };
        assert!(reach.reads(&folder.join("worldpass").join("cam.json")));
        assert!(!reach.reads(&dir.join("elsewhere.json")));
        assert!(!reach.reads(&folder.join("..").join("elsewhere.json")));

        if cfg!(windows) {
            for refused in [
                r"\\server\share\x.json",
                r"\\?\UNC\server\share\x.json",
                r"\\.\pipe\x",
                r"\??\C:\x.json",
                r"C:\x\NUL.txt",
                r"C:\x\a.json:hidden",
            ] {
                assert!(plain(Path::new(refused)).is_err(), "{refused}");
            }
            assert!(plain(Path::new(r"C:\x\a.json")).is_ok());
            assert_eq!(script_folder(Path::new(r"C:\go.jsx")), None);
        } else {
            assert_eq!(script_folder(Path::new("/go.jsx")), None);
        }
        let home = std::env::home_dir().unwrap();
        assert_eq!(script_folder(&home.join("go.jsx")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
