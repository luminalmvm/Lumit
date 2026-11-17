//! LUT files kept in the user's library, found for a row whose file isn't on
//! this machine.
//!
//! A project names a LUT by the path it had where it was made. Opened on
//! another computer, that path is usually missing, while the person's own LUTs
//! follow them in the library. So a LUT or OCIO row whose file is missing reads
//! the file of the same name from the library's LUT folder instead. The
//! project keeps its own path, so it opens the same way everywhere.
//!
//! Runs on whichever thread names or draws a frame. The folder is listed once
//! and listed again only when one of its folders changes, so naming a frame
//! costs a stat per folder, not a walk.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

/// What the LUT and OCIO effects read.
const EXTENSIONS: [&str; 8] = ["cube", "spi1d", "spi3d", "clf", "ctf", "cc", "ccc", "cdl"];

/// The library's LUT files as last listed, with every folder the listing
/// walked and when each last changed. One entry, replaced whole.
struct Listing {
    folders: Vec<(PathBuf, Option<SystemTime>)>,
    files: Vec<PathBuf>,
}

static LISTING: Mutex<Option<Listing>> = Mutex::new(None);

/// The library's file to read for the row file `path`, when `path` isn't
/// there and the library holds a LUT of the same file name: the top level's
/// first, then the sub-folders', by path, so the answer is the same every
/// time. `None` when the file is where the row says, or nowhere.
#[must_use]
pub fn found(path: &str) -> Option<String> {
    // The name after either separator, since a project made on Windows
    // names its files with back slashes.
    let name = path.rsplit(['/', '\\']).next()?;
    let extension = Path::new(name).extension()?.to_str()?;
    if !EXTENSIONS.iter().any(|e| extension.eq_ignore_ascii_case(e)) || Path::new(path).exists() {
        return None;
    }
    let folder = lumit_project::library_home::luts_dir()?;
    let files = listing(&folder);
    files
        .into_iter()
        .find(|f| {
            f.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .map(|f| f.to_string_lossy().into_owned())
}

/// [`found`], or `path` itself.
#[must_use]
pub fn or_found(path: &str) -> String {
    found(path).unwrap_or_else(|| path.to_owned())
}

/// The LUT files in `folder`, from the listing kept while none of its folders
/// has changed. The lock is let go of while the folder is walked.
fn listing(folder: &Path) -> Vec<PathBuf> {
    let changed = |stamp: &(PathBuf, Option<SystemTime>)| {
        std::fs::metadata(&stamp.0).and_then(|m| m.modified()).ok() != stamp.1
    };
    if let Some(kept) = LISTING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .filter(|l| l.folders.first().is_some_and(|(root, _)| root == folder))
        .filter(|l| !l.folders.iter().any(changed))
    {
        return kept.files.clone();
    }
    let mut walked = Listing {
        folders: Vec::new(),
        files: Vec::new(),
    };
    walk(folder, 0, &mut walked);
    let files = walked.files.clone();
    *LISTING.lock().unwrap_or_else(PoisonError::into_inner) = Some(walked);
    files
}

fn walk(folder: &Path, depth: usize, out: &mut Listing) {
    let stamp = std::fs::metadata(folder).and_then(|m| m.modified()).ok();
    out.folders.push((folder.to_path_buf(), stamp));
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<(PathBuf, std::fs::FileType)> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| Some((e.path(), e.file_type().ok()?)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    // This folder's files before any folder inside it.
    for (path, kind) in &entries {
        if kind.is_file() {
            out.files.push(path.clone());
        }
    }
    // Deep enough for any library; a link loop is not followed anyway.
    if depth < 16 {
        for (path, kind) in &entries {
            if kind.is_dir() {
                walk(path, depth + 1, out);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use lumit_core::model::{EffectValue, FileParam};

    /// **A LUT row whose file is missing draws with the library's LUT of the
    /// same name.** A white solid through a cube that takes green to nought
    /// comes out magenta; without the library's copy it would stay white.
    #[test]
    fn a_missing_lut_draws_with_the_one_in_the_library() {
        let Ok(mut r) = crate::headless::HeadlessRenderer::shared() else {
            lumit_gpu::no_adapter();
            return;
        };
        let home = tempfile::tempdir().unwrap();
        let luts = home.path().join("luts").join("Grades");
        std::fs::create_dir_all(&luts).unwrap();
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for i in 0..8u32 {
            cube += &format!("{} 0 {}\n", i & 1, (i >> 2) & 1);
        }
        std::fs::write(luts.join("No green.cube"), cube).unwrap();

        let mut doc = lumit_core::model::Document::new();
        let settings = lumit_core::edits::CompSettings {
            width: 8,
            height: 8,
            ..Default::default()
        };
        let (comp_id, ops) = lumit_core::edits::new_comp_ops(&doc, settings, None);
        for op in &ops {
            lumit_core::ops::apply(&mut doc, op).unwrap();
        }
        let comp = doc.comp(comp_id).unwrap().clone();
        let (mut layer, ops) = lumit_core::edits::white_solid_layer(&doc, &comp);
        for op in &ops {
            lumit_core::ops::apply(&mut doc, op).unwrap();
        }
        let mut lut = lumit_core::fx::builtins::instantiate("lut").unwrap();
        let missing = home.path().join("elsewhere").join("No green.cube");
        for p in &mut lut.params {
            if p.id == "file" {
                p.value = EffectValue::File(FileParam::single(missing.to_string_lossy()));
            }
        }
        layer.effects.push(lut);
        doc.comp_mut(comp_id).unwrap().layers.push(layer);
        let doc = std::sync::Arc::new(doc);

        lumit_project::library_home::set_library_home(Some(home.path().to_path_buf()));
        let drawn = r.render_rgba(&doc, comp_id, 0, 1.0);
        lumit_project::library_home::set_library_home(None);
        let (rgba, _, _) = drawn.unwrap();
        let middle = &rgba[(4 * 8 + 4) * 4..][..4];
        assert!(
            middle[0] > 200 && middle[1] < 10 && middle[2] > 200,
            "drawn without the library's LUT: {middle:?}"
        );
    }
}
