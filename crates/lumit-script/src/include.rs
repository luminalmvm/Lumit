//! After Effects' `#include`: another file read into the script where the
//! line names it, before the script runs.
//!
//! A name is read from the including file's folder, then from the folders a
//! `#includepath` line gave. Only files in the script's own folder and the
//! folders inside it come in, by the rule reads keep to, and each comes in
//! once. `//@include` is the same line written so other programs read it as a
//! comment. After Effects' other `#` lines, such as `#target`, are blanked.
//!
//! Lines keep their numbers through a map of where each run of lines came
//! from, so an error in an included file names that file and its line.

use std::path::{Path, PathBuf};

use crate::reach::{self, Reach};
use crate::ScriptError;

/// How deep includes may go, and how much text they may come to.
const MAX_DEPTH: usize = 16;
const MAX_BYTES: usize = 16 << 20;

/// A script with its includes read in.
pub(crate) struct Source {
    pub(crate) text: String,
    /// Where each run of lines in `text` came from, in order.
    pieces: Vec<Piece>,
    /// Lines written so far.
    lines: usize,
}

struct Piece {
    /// Its first line in the text, counted from 1.
    start: usize,
    spot: Spot,
}

/// A line in the script or one of its includes.
#[derive(Clone, Default)]
struct Spot {
    /// The included file, as a path from the script's folder. `None` is the
    /// script itself.
    file: Option<String>,
    line: usize,
    /// The script's own line the include came in on.
    top: usize,
}

impl Source {
    /// Line `line` of the text as the script's own line, and `error` saying
    /// which included file and line it is in when it is in one, which the
    /// last answer says.
    pub(crate) fn locate(&self, error: ScriptError, line: usize) -> (ScriptError, usize, bool) {
        let spot = self
            .pieces
            .iter()
            .rev()
            .find(|p| p.start <= line)
            .map(|p| Spot {
                line: p.spot.line + (line - p.start),
                ..p.spot.clone()
            })
            .unwrap_or_default();
        let included = spot.file.is_some();
        let (error, line) = spot.say(error);
        (error, line, included)
    }

    fn mark(&mut self, file: Option<&str>, line: usize, top: usize) {
        self.pieces.push(Piece {
            start: self.lines + 1,
            spot: Spot {
                file: file.map(str::to_owned),
                line,
                top,
            },
        });
    }

    fn push(&mut self, line: &str) {
        self.text.push_str(line);
        if !line.ends_with('\n') {
            self.text.push('\n');
        }
        self.lines += 1;
    }
}

impl Spot {
    /// `error` at this spot, and the script's own line to show it on.
    fn say(self, error: ScriptError) -> (ScriptError, usize) {
        let Some(file) = self.file else {
            return (error, self.line);
        };
        let at = format!("{file}, line {}", self.line);
        let error = match error {
            ScriptError::BadArgs(m) => ScriptError::BadArgs(format!("{at}: {m}")),
            ScriptError::NotFound(m) => ScriptError::NotFound(format!("{at}: {m}")),
            ScriptError::Failed(m) => ScriptError::Failed(format!("{at}: {m}")),
            other => other,
        };
        (error, self.top)
    }
}

/// `source` with its includes read in. `script` is the script's own file and
/// `folder` the one it may read in, when it has them. An error comes with the
/// script's own line.
pub(crate) fn expand(
    source: &str,
    script: Option<&Path>,
    folder: Option<&Path>,
) -> Result<Source, (ScriptError, usize)> {
    let here = script
        .and_then(|s| std::path::absolute(s).ok())
        .and_then(|s| s.parent().map(Path::to_path_buf));
    let mut splicer = Splicer {
        folder,
        out: Source {
            text: String::with_capacity(source.len()),
            pieces: Vec::new(),
            lines: 0,
        },
        seen: script
            .and_then(|s| s.canonicalize().ok())
            .into_iter()
            .collect(),
        paths: Vec::new(),
        bytes: 0,
    };
    splicer
        .splice(source, here.as_deref(), None, 0, 0)
        .map_err(|(error, spot)| spot.say(error))?;
    Ok(splicer.out)
}

struct Splicer<'a> {
    folder: Option<&'a Path>,
    out: Source,
    /// Files read in so far, as real paths.
    seen: Vec<PathBuf>,
    /// Where `#includepath` says to look.
    paths: Vec<PathBuf>,
    /// What the includes have added, in bytes.
    bytes: usize,
}

impl Splicer<'_> {
    /// Write `text`, from `file` in folder `here`, with its includes read in.
    fn splice(
        &mut self,
        text: &str,
        here: Option<&Path>,
        file: Option<&str>,
        top: usize,
        depth: usize,
    ) -> Result<(), (ScriptError, Spot)> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        self.out.mark(file, 1, top);
        for (n, line) in text.split_inclusive('\n').enumerate() {
            let n = n + 1;
            // The script's own include lines are the ones its errors point at.
            let top = if file.is_none() { n } else { top };
            let spot = || Spot {
                file: file.map(str::to_owned),
                line: n,
                top,
            };
            match directive(line) {
                Some(("include", name)) => {
                    let Some(here) = here.filter(|_| self.folder.is_some()) else {
                        return Err((
                            ScriptError::BadArgs(
                                "This script has no folder of its own, so it cannot include files."
                                    .into(),
                            ),
                            spot(),
                        ));
                    };
                    let path = self.find(name, here).map_err(|e| (e, spot()))?;
                    let real = path.canonicalize().map_err(|_| {
                        let e = ScriptError::NotFound(format!("{name} could not be found."));
                        (e, spot())
                    })?;
                    if self.seen.contains(&real) {
                        self.out.push("\n");
                        continue;
                    }
                    if depth >= MAX_DEPTH {
                        let e = format!("Includes go at most {MAX_DEPTH} deep.");
                        return Err((ScriptError::BadArgs(e), spot()));
                    }
                    self.seen.push(real.clone());
                    let left = MAX_BYTES.saturating_sub(self.bytes) as u64;
                    let inner = lumit_ingress::read_to_string_capped(&real, left)
                        .map_err(|e| (ScriptError::Failed(e.to_string()), spot()))?;
                    self.bytes += inner.len();
                    let shown = self
                        .folder
                        .and_then(|f| path.strip_prefix(f).ok())
                        .unwrap_or(Path::new(name))
                        .display()
                        .to_string();
                    self.splice(&inner, path.parent(), Some(&shown), top, depth + 1)?;
                    self.out.mark(file, n + 1, top);
                }
                Some(("includepath", paths)) => {
                    if let Some(here) = here {
                        self.paths = paths
                            .split(';')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(|p| here.join(p))
                            .collect();
                    }
                    self.out.push("\n");
                }
                // Not JavaScript, and nothing Lumit needs.
                Some(("target" | "targetengine" | "strict" | "script" | "engine", _)) => {
                    self.out.push("\n");
                }
                _ => self.out.push(line),
            }
        }
        Ok(())
    }

    /// The file `name` names, read from `here` first and then the include
    /// paths. Asked of the reach before the file system, so a file out of
    /// reach is refused the same whether it is there or not.
    fn find(&self, name: &str, here: &Path) -> Result<PathBuf, ScriptError> {
        let reach = Reach {
            folder: self.folder.map(Path::to_path_buf),
            ..Reach::default()
        };
        let wanted = Path::new(name);
        let mut places = vec![here.join(wanted)];
        if wanted.is_relative() {
            places.extend(self.paths.iter().map(|p| p.join(wanted)));
        }
        let mut reached = Vec::new();
        for place in places {
            if reach.reads(reach::plain(&place)?) {
                reached.push(place);
            }
        }
        if reached.is_empty() {
            return Err(ScriptError::BadArgs(format!(
                "A script can only include files in its own folder, and {name} is not in it."
            )));
        }
        let first = reached.first().cloned();
        reached
            .into_iter()
            .find(|p| p.is_file())
            .or(first)
            .ok_or_else(|| ScriptError::NotFound(format!("{name} could not be found.")))
    }
}

/// The `#` line's word and what follows it, quotes and a closing `;` taken
/// off: `#include "lib.jsx"` is `("include", "lib.jsx")`.
fn directive(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let rest = line
        .strip_prefix('#')
        .or_else(|| line.strip_prefix("//@"))?;
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .unwrap_or(rest.len());
    let (word, arg) = rest.split_at(end);
    let arg = arg.trim().trim_end_matches(';').trim_end();
    Some((word, arg.trim_matches(|c| c == '"' || c == '\'')))
}
