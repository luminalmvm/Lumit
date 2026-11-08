//! What both readers share: the budget, finding the files a model names, and
//! decoding its pictures.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use lumit_ingress::Budget;

use crate::{Image, SceneError};

/// Roughly what one vertex costs with every attribute filled, so a mesh is
/// charged before it is read rather than after.
pub(crate) const VERTEX_BYTES: usize = 128;

/// One read in progress: the file, the budget it spends and the flag that
/// stops it.
pub(crate) struct Reader<'a> {
    pub path: &'a Path,
    pub dir: PathBuf,
    pub budget: Budget,
    pub cancel: &'a AtomicBool,
    pub missing: Vec<String>,
}

impl<'a> Reader<'a> {
    pub fn new(path: &'a Path, budget: Budget, cancel: &'a AtomicBool) -> Self {
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Self {
            path,
            dir,
            budget,
            cancel,
            missing: Vec::new(),
        }
    }

    pub fn fail(&self, reason: impl ToString) -> SceneError {
        SceneError::Read {
            path: self.path.to_path_buf(),
            reason: reason.to_string(),
        }
    }

    pub fn too_large(&self) -> SceneError {
        SceneError::TooLarge {
            path: self.path.to_path_buf(),
        }
    }

    pub fn check(&self) -> Result<(), SceneError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(SceneError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Charge `bytes` to the budget, and stop if the user has given up.
    pub fn take(&mut self, bytes: usize) -> Result<(), SceneError> {
        self.check()?;
        self.budget
            .take_bytes(bytes as u64)
            .map_err(|_| self.too_large())
    }

    /// Read a whole file, never more than the budget has left.
    pub fn read_file(&mut self, file: &Path) -> Result<Vec<u8>, SceneError> {
        let bytes = lumit_ingress::read_capped(file, self.budget.bytes_remaining()).map_err(
            |e| match e {
                lumit_ingress::IngressError::Io { .. } => self.fail(e),
                _ => self.too_large(),
            },
        )?;
        self.take(bytes.len())?;
        Ok(bytes)
    }

    /// The bytes a model names by URI or path: inline data, or a file found
    /// by [`find`]. A file that can't be found is noted in `missing`.
    pub fn fetch(&mut self, uri: &str) -> Result<Option<Vec<u8>>, SceneError> {
        if let Some(data) = uri.strip_prefix("data:") {
            let Some((_, encoded)) = data.split_once(";base64,") else {
                return Err(self.fail("an inline file isn't base64"));
            };
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| self.fail(e))?;
            self.take(bytes.len())?;
            return Ok(Some(bytes));
        }
        match find(&self.dir, &unescape(uri)) {
            Some(file) => self.read_file(&file).map(Some),
            None => {
                self.missing.push(uri.to_string());
                Ok(None)
            }
        }
    }

    /// A picture's bytes decoded to RGBA8, or `None` when they aren't a
    /// picture Lumit reads, which notes it in `missing` as `name`. Only
    /// running out of budget is an error.
    pub fn decode(&mut self, bytes: &[u8], name: &str) -> Result<Option<Image>, SceneError> {
        self.check()?;
        let Ok(mut reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()
        else {
            self.missing.push(name.to_string());
            return Ok(None);
        };
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(self.budget.bytes_remaining());
        reader.limits(limits);
        let picture = match reader.decode() {
            Ok(picture) => picture.into_rgba8(),
            Err(image::ImageError::Limits(_)) => return Err(self.too_large()),
            Err(_) => {
                self.missing.push(name.to_string());
                return Ok(None);
            }
        };
        self.take(picture.len())?;
        Ok(Some(Image {
            width: picture.width(),
            height: picture.height(),
            rgba8: picture.into_raw(),
        }))
    }
}

/// Where a file a model names really is. Models often carry paths from the
/// machine they were made on, so after the path itself this looks for the
/// file's name beside the model, in the folders beside it, and in the folders
/// beside the model's own folder.
pub(crate) fn find(dir: &Path, named: &str) -> Option<PathBuf> {
    let as_named = Path::new(named);
    let direct = if as_named.is_absolute() {
        as_named.to_path_buf()
    } else {
        dir.join(as_named)
    };
    if direct.is_file() {
        return Some(direct);
    }
    // Split on both slashes: a path written on Windows still reads elsewhere.
    let name = named.rsplit(['/', '\\']).next().filter(|n| !n.is_empty())?;
    let beside = dir.join(name);
    if beside.is_file() {
        return Some(beside);
    }
    let folders = |d: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    };
    let mut places = folders(dir);
    if let Some(up) = dir.parent() {
        places.extend(folders(up));
    }
    places.sort();
    places
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// A URI's `%20`-style escapes, undone. Anything malformed is left as it is.
pub(crate) fn unescape(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| uri.to_string())
}

/// The `mtllib` files an OBJ names, as written.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn obj_libraries(obj: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(obj)
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("mtllib"))
        .map(|rest| rest.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}
